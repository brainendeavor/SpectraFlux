# Durable Steps, Resilience & Causal Monotonicity

This document details the resilience patterns built into **SpectraFlux**: 80/20 durable step memoization, exponential backoff with dead-letter queue (DLQ) routing, and monotonic Hybrid Logical Clock (HLC) causal guards.

---

## 1. The 80/20 Durable Step Checkpointing Pattern

In distributed event processing, guest workers often perform non-idempotent side effects (e.g. charging a customer via Stripe, sending an SMS, or reserving stock via an external warehouse API). If the worker crashes or times out *after* the external call, the message broker redelivers the event, risking **duplicate charges or actions**.

SpectraFlux provides an **80/20 durable step primitive** that eliminates 80% of workflow complexity without requiring a heavy multi-cluster workflow engine.

```
Incoming Event (UUIDv7 Command ID: 0191b2c4-8840-7ac3...)
         │
         ▼
[ Step: "stripe_charge" ]
         │
         ├── Check Host KV: "chk:0191b2c4...:stripe_charge"
         │
         ├── Found? ──► Return Cached JSON (Skip Execution!)
         │
         └── Not Found? ──► Execute Side Effect Function
                                  │
                                  ▼
                            Save to Host KV with TTL
                                  │
                                  ▼
                            Proceed to Next Step
```

### TypeScript Usage
```typescript
import { EventContext, EventVerdict } from "@spectraflux/sdk";

export class CheckoutCell extends Fluxcell {
  handleEvent(ctx: EventContext): string {
    // Step 1: External payment (executed exactly once)
    const paymentJson = ctx.step("charge_card", (): string => {
      // Third-party HTTP call or mutation
      return JSON.stringify({ chargeId: "ch_123", status: "success" });
    });

    // Step 2: Database record commit
    const db = Database.default();
    const tx = db.beginTx();
    tx.execute("INSERT INTO orders (payment_data) VALUES ($1)", [paymentJson]);
    tx.commit();

    return EventVerdict.Ack.toJson();
  }
}
```

### Host Key Storage Format & Lifecycle
Step results are persisted in the host key-value store (`internal_storage`) with the format:
```
chk:<command_uuidv7>:<step_name>
```
With a configurable time-to-live (default: **86,400 seconds / 24 hours**).

#### Incomplete Execution & Retry Re-Entry
When a multi-step mutation fails midway (e.g., Step 1 succeeds, Step 2 succeeds, but Step 3 fails or times out):
1. **Preserved Intermediate State**: The checkpoint keys for Steps 1 and 2 are **retained in storage**. They are *not* discarded or rolled back upon handler failure.
2. **Exponential Backoff NACK**: The chassis emits a `NACK` with exponential backoff delay back to the broker.
3. **Fast Re-Entry (< 50µs)**: When the broker redelivers the event, the fluxcell re-executes with the same `command_id`. As execution hits Step 1 and Step 2, `ctx.step` detects the existing cached outputs, skips execution, and returns the cached JSON in **under 50 microseconds**. Execution resumes directly at Step 3 without repeating prior external side-effects.

#### Why You Cannot Side-Step This With Multiple Queues
In enterprise workflows, downstream database updates are often **causally dependent on external API outputs** (e.g. recording an order in PostgreSQL requires the Stripe transaction ID returned by the payment step). Because internal state transitions depend on external API outputs, decoupling them into independent fire-and-forget queue listeners causes severe partial-state hazards if the external call fails. `ctx.step` provides an ordered, dependent pipeline inside a single unit of execution with automatic memoization across retries.

---

## 2. Level 1 Resilience: Exponential Backoff & DLQ Routing

SpectraFlux implements an automated resilience pipeline for all broker-delivered events:

```
                      Broker Message Received
                                │
                                ▼
                       Execute Guest Handler
                                │
            ┌───────────────────┴───────────────────┐
            ▼                                       ▼
      Success (ACK)                          Failure / Error
            │                                       │
      Commit to Broker                      Increment Retry Count
                                                    │
                                   ┌────────────────┴────────────────┐
                                   ▼                                 ▼
                           retries < max_retries            retries >= max_retries
                                   │                                 │
                           Wait with Backoff                Publish to DLQ Topic
                           (Exponential + Jitter)           "dlq.<topic_name>"
                                   │                                 │
                           Re-execute Handler               Acknowledge Original Msg
```

### Resilience Configuration (`spectra-flux.toml`)
```toml
[resilience]
max_retries = 3
initial_backoff_ms = 100
max_backoff_ms = 5000
backoff_factor = 2.0
jitter = true
dlq_topic_prefix = "dlq."
```

### DLQ Envelope & Checkpoint Eviction
When an event exhausts its retries (`msg.delivery_attempt >= max_retries`) or if the fluxcell explicitly returns `event_verdict::dead_letter`, the chassis formats a standardized diagnostic envelope:
```json
{
  "messageId": "0191b2c4-8840-7ac3-...",
  "originalTopic": "orders.checkout",
  "payload": { ... },
  "deliveryAttempts": 3,
  "reason": "Fluxcell 'checkout-cell' failed: Stripe API rate limit exceeded",
  "failedAt": "2026-09-23T19:20:00Z"
}
```
* **Primary Stream Unblocking**: The dead-lettered message is published to `dlq.<topic>` and the original message is acknowledged (`ACK`) off the primary stream, preventing head-of-line blocking.
* **Storage Eviction**: Any intermediate checkpoints recorded during prior failed attempts naturally expire after their 24-hour TTL in `internal_storage`, leaving zero orphaned disk state.

### Circuit Breakers
If a downstream dependency (such as PostgreSQL) fails consistently:
1. The circuit breaker transitions from `Closed` to `Open`.
2. Subsequent events are immediately NACKed or rejected without invoking the WebAssembly guest, protecting memory pools and preventing worker exhaustion.
3. After a recovery interval, the circuit enters `HalfOpen` to test backend connectivity with canary invocations.

---

## 3. Causal Monotonicity & HLC Watermarking

Distributed message brokers (Kafka, NATS) guarantee at-least-once delivery, but network partitions or multi-partition topics can cause events to arrive **out of order**. If an older update arrives after a newer update, standard database writes could overwrite newer state with stale data.

SpectraFlux solves this using **Hybrid Logical Clocks (HLC)** generated by SpectraGQL (`<physical_millis>.<logical_counter>`).

### In-Memory Causality Guard (`CausalGuard`)
Protects in-memory aggregates using a bounded LRU high-water mark cache:

```rust
use fluxcell_sdk::prelude::*;

static GUARD: std::sync::LazyLock<CausalGuard<String>> = 
    std::sync::LazyLock::new(|| CausalGuard::new(10_000));

let verdict = GUARD.evaluate_and_advance(&entity_id, &event.hlc);
if verdict.should_discard() {
    // Event is causally stale; safely discard and acknowledge
    return EventVerdict::IgnoredStaleHlc.to_json();
}
```

### Database High-Water Mark Table
For distributed clusters, state is persisted in a database watermark table:

```sql
CREATE TABLE IF NOT EXISTS flux_watermarks (
    aggregate_type VARCHAR(64) NOT NULL,
    aggregate_id VARCHAR(128) NOT NULL,
    high_water_hlc VARCHAR(64) NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (aggregate_type, aggregate_id)
);
```

The SDK's `advance_db_watermark` function atomically validates and updates the clock within the host transaction:
```rust
let verdict = advance_db_watermark(&mut tx, "accounts", &account_id, &event.hlc)?;
if verdict.should_discard() {
    tx.rollback();
    return Ok(EventVerdict::IgnoredStaleHlc);
}
```
If an event with an older HLC arrives, it is safely ignored with zero side effects.
