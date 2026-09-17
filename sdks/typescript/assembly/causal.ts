// AssemblyScript SDK: Causal Monotonicity & Watermark Engine

import { FluxTx } from "./db";
import { parse_hlc } from "./hlc";

export enum CausalVerdict {
  Fresh = 0,
  Stale = 1,
  Duplicate = 2,
}

export function is_fresh(verdict: CausalVerdict): bool {
  return verdict == CausalVerdict.Fresh;
}

export function should_discard(verdict: CausalVerdict): bool {
  return verdict != CausalVerdict.Fresh;
}

/**
 * Compares two HLC timestamps (`incoming` vs `existing`) returning a precise `CausalVerdict`.
 */
export function evaluate_causality(incoming: string, existing: string): CausalVerdict {
  const inc = parse_hlc(incoming);
  const ext = parse_hlc(existing);

  if (inc != null && ext != null) {
    if (inc.wall < ext.wall) {
      return CausalVerdict.Stale;
    } else if (inc.wall > ext.wall) {
      return CausalVerdict.Fresh;
    } else if (inc.counter < ext.counter) {
      return CausalVerdict.Stale;
    } else if (inc.counter > ext.counter) {
      return CausalVerdict.Fresh;
    } else {
      return CausalVerdict.Duplicate;
    }
  }

  if (incoming < existing) {
    return CausalVerdict.Stale;
  } else if (incoming > existing) {
    return CausalVerdict.Fresh;
  } else {
    return CausalVerdict.Duplicate;
  }
}

/**
 * A fast, bounded in-memory causal gate tracking high-water marks per entity.
 *
 * Discards duplicate and out-of-order deliveries in guest memory before dispatching
 * expensive database transactions.
 */
export class CausalGuard {
  private capacity: i32;
  private watermarks: Map<string, string>;
  private keys: Array<string>;

  constructor(capacity: i32 = 1000) {
    this.capacity = capacity;
    this.watermarks = new Map<string, string>();
    this.keys = new Array<string>();
  }

  /**
   * Evaluates an incoming HLC timestamp against the entity's current watermark.
   *
   * If `Fresh`, automatically advances the watermark to `incomingHlc`.
   * If `Stale` or `Duplicate`, leaves the watermark untouched.
   */
  evaluateAndAdvance(key: string, incomingHlc: string): CausalVerdict {
    if (this.watermarks.has(key)) {
      const existing = this.watermarks.get(key);
      const verdict = evaluate_causality(incomingHlc, existing);
      if (!is_fresh(verdict)) {
        return verdict;
      }
    } else {
      if (this.watermarks.size >= this.capacity) {
        if (this.keys.length > 0) {
          const oldest = this.keys.shift();
          this.watermarks.delete(oldest);
        }
      }
      this.keys.push(key);
    }

    this.watermarks.set(key, incomingHlc);
    return CausalVerdict.Fresh;
  }

  getWatermark(key: string): string | null {
    if (this.watermarks.has(key)) {
      return this.watermarks.get(key);
    }
    return null;
  }

  clear(): void {
    this.watermarks.clear();
    this.keys = new Array<string>();
  }

  size(): i32 {
    return this.watermarks.size;
  }
}

/**
 * SQL DDL to create the auxiliary causal watermark table for schemas that lack native HLC columns.
 */
export const ENSURE_WATERMARK_TABLE_SQL: string =
  "CREATE TABLE IF NOT EXISTS _flux_causal_watermarks (" +
  "namespace VARCHAR(64) NOT NULL," +
  "entity_id VARCHAR(128) NOT NULL," +
  "hlc VARCHAR(64) NOT NULL," +
  "updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()," +
  "PRIMARY KEY (namespace, entity_id)" +
  ");";

/**
 * Atomically evaluates and advances a causal high-water mark inside a host DB transaction.
 *
 * This allows existing database tables that only maintain updated_at timestamps
 * to achieve strict monotonic causal consistency without altering their schema.
 */
export function advance_db_watermark(
  tx: FluxTx,
  namespace: string,
  entityId: string,
  incomingHlc: string
): CausalVerdict {
  // 1. Query current watermark inside transaction
  const querySql = "SELECT hlc FROM _flux_causal_watermarks WHERE namespace = $1 AND entity_id = $2";
  const paramsJson = '["' + namespace + '","' + entityId + '"]';
  const rowsJson = tx.query(querySql, paramsJson);

  // Check if existing HLC is in rows
  const hlcIdx = rowsJson.indexOf('"hlc":');
  if (hlcIdx != -1) {
    const start = rowsJson.indexOf('"', hlcIdx + 6);
    if (start != -1) {
      const end = rowsJson.indexOf('"', start + 1);
      if (end != -1) {
        const existingHlc = rowsJson.substring(start + 1, end);
        const verdict = evaluate_causality(incomingHlc, existingHlc);
        if (!is_fresh(verdict)) {
          return verdict;
        }
      }
    }
  }

  // 2. Upsert the fresh high-water mark with conditional monotonic conflict protection
  const upsertSql =
    "INSERT INTO _flux_causal_watermarks (namespace, entity_id, hlc, updated_at) " +
    "VALUES ($1, $2, $3, NOW()) " +
    "ON CONFLICT (namespace, entity_id) DO UPDATE " +
    "SET hlc = EXCLUDED.hlc, updated_at = NOW() " +
    "WHERE CASE WHEN _flux_causal_watermarks.hlc LIKE '%.%' AND EXCLUDED.hlc LIKE '%.%' THEN " +
    "(split_part(_flux_causal_watermarks.hlc, '.', 1)::bigint < split_part(EXCLUDED.hlc, '.', 1)::bigint OR " +
    "(split_part(_flux_causal_watermarks.hlc, '.', 1)::bigint = split_part(EXCLUDED.hlc, '.', 1)::bigint AND " +
    "split_part(_flux_causal_watermarks.hlc, '.', 2)::bigint < split_part(EXCLUDED.hlc, '.', 2)::bigint)) " +
    "ELSE _flux_causal_watermarks.hlc < EXCLUDED.hlc END";

  const upsertParams = '["' + namespace + '","' + entityId + '","' + incomingHlc + '"]';
  tx.execute(upsertSql, upsertParams);

  return CausalVerdict.Fresh;
}
