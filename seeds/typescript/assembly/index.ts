// Canonical Barebones TypeScript Starter Fluxcell for SpectraFlux
// Built on the official @spectraflux/sdk

import {
  Fluxcell,
  FluxcellMetadata,
  HttpRequest,
  HttpResponse,
  RouteMeta,
  EventContext,
  TelemetryBuffer,
  DeduplicationBuffer,
  CausalGuard,
  CausalVerdict,
  registerFluxcell,
} from "@spectraflux/sdk/assembly/index";

// In-guest telemetry buffer tracking metrics and logs
const telemetry = new TelemetryBuffer(100);

// In-memory monotonic causal high-water mark guard
const causalGuard = new CausalGuard(1000);

// Deduplication buffer for idempotency
const dedup = new DeduplicationBuffer<string>(1000);

export class StarterFluxcell extends Fluxcell {
  metadata(): FluxcellMetadata {
    return new FluxcellMetadata("0.1.0", "Barebones Starter Fluxcell");
  }

  routes(): RouteMeta[] {
    return [
      RouteMeta.get("/health", "Health check endpoint"),
      RouteMeta.post("/echo", "Echoes incoming payload or executes transactional command"),
      RouteMeta.get("/stats", "Retrieves in-guest telemetry metrics"),
    ];
  }

  subscriptions(): string[] {
    return ["events.sample"];
  }

  handleHttp(req: HttpRequest): HttpResponse {
    telemetry.recordProcessed();

    if (req.path == "/health" || req.path == "/stats") {
      return HttpResponse.json(telemetry.statsJson("fluxcell-seed-ts", "0.1.0"));
    } else if (req.path == "/echo") {
      return HttpResponse.json('{"status":"ok","echo":"' + req.body + '"}');
    }
    return HttpResponse.notFound("Endpoint not found: " + req.path);
  }

  handleEvent(event: EventContext): string {
    // 1. Monotonic Causal Guard: discard stale or duplicate HLC deliveries
    if (event.hlc.length > 0) {
      const verdict = causalGuard.evaluateAndAdvance(event.topic, event.hlc);
      if (verdict == CausalVerdict.Duplicate || verdict == CausalVerdict.Stale) {
        telemetry.recordStale();
        telemetry.log("WARN", "Discarded stale event for topic: " + event.topic, event.hlc);
        return '{"status":"discarded","reason":"stale_or_duplicate_hlc"}';
      }
    }

    telemetry.recordProcessed();
    telemetry.log("INFO", "Successfully processed event on topic: " + event.topic, event.hlc);
    return '{"status":"processed","topic":"' + event.topic + '"}';
  }
}

// Register the starter cell instance with the SDK runtime
registerFluxcell(new StarterFluxcell());

// Re-export C-ABI functions required by SpectraFlux Wasmtime host
export {
  allocate,
  deallocate,
  get_metadata,
  get_subscriptions,
  get_routes,
  handle_http,
  handle_event,
} from "@spectraflux/sdk/assembly/index";
