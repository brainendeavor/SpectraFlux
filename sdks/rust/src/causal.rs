//! Causal Monotonicity & Watermark Engine
//!
//! Provides deterministic causality checks and monotonic guards for distributed
//! event streams where target databases only maintain low-precision `updated_at`
//! timestamps or cannot be modified to store Hybrid Logical Clock (HLC) metadata.

use crate::db::Transaction;
use crate::hlc::parse_hlc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::sync::Mutex;

/// The verdict returned when evaluating an incoming HLC timestamp against a causal watermark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CausalVerdict {
    /// The event is strictly causally newer than any previously observed event for this entity.
    Fresh,
    /// The event is older than the entity's current high-water mark (out-of-order delivery).
    Stale,
    /// The event has an identical HLC timestamp as the entity's current high-water mark (duplicate redelivery).
    Duplicate,
}

impl CausalVerdict {
    /// Returns `true` if the event is strictly newer and should be processed.
    #[inline]
    pub fn is_fresh(&self) -> bool {
        matches!(self, Self::Fresh)
    }

    /// Returns `true` if the event should be discarded without mutating state.
    #[inline]
    pub fn should_discard(&self) -> bool {
        !self.is_fresh()
    }
}

/// Compares two HLC timestamps (`incoming` vs `existing`) returning a precise `CausalVerdict`.
pub fn evaluate_causality(incoming: &str, existing: &str) -> CausalVerdict {
    let inc = parse_hlc(incoming);
    let ext = parse_hlc(existing);

    match (inc, ext) {
        (Some((w_inc, c_inc)), Some((w_ext, c_ext))) => {
            if w_inc < w_ext {
                CausalVerdict::Stale
            } else if w_inc > w_ext {
                CausalVerdict::Fresh
            } else if c_inc < c_ext {
                CausalVerdict::Stale
            } else if c_inc > c_ext {
                CausalVerdict::Fresh
            } else {
                CausalVerdict::Duplicate
            }
        }
        _ => {
            if incoming < existing {
                CausalVerdict::Stale
            } else if incoming > existing {
                CausalVerdict::Fresh
            } else {
                CausalVerdict::Duplicate
            }
        }
    }
}

/// A fast, bounded in-memory causal gate tracking high-water marks per entity.
///
/// Discards duplicate and out-of-order deliveries in guest memory before dispatching
/// expensive database transactions.
pub struct CausalGuard<K> {
    capacity: usize,
    watermarks: Mutex<(HashMap<K, String>, VecDeque<K>)>,
}

impl<K: Hash + Eq + Clone> CausalGuard<K> {
    /// Creates a new causal guard with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            watermarks: Mutex::new((HashMap::new(), VecDeque::new())),
        }
    }

    /// Evaluates an incoming HLC timestamp against the entity's current watermark.
    ///
    /// If `Fresh`, automatically advances the watermark to `incoming_hlc`.
    /// If `Stale` or `Duplicate`, leaves the watermark untouched.
    pub fn evaluate_and_advance(&self, key: &K, incoming_hlc: &str) -> CausalVerdict {
        let mut guard = self.watermarks.lock().unwrap_or_else(|e| e.into_inner());
        let (ref mut map, ref mut queue) = *guard;
        if let Some(existing) = map.get(key) {
            let verdict = evaluate_causality(incoming_hlc, existing);
            if !verdict.is_fresh() {
                return verdict;
            }
        } else {
            if map.len() >= self.capacity {
                if let Some(oldest) = queue.pop_front() {
                    map.remove(&oldest);
                }
            }
            queue.push_back(key.clone());
        }

        map.insert(key.clone(), incoming_hlc.to_string());
        CausalVerdict::Fresh
    }

    /// Returns the current high-water mark for `key` if tracked.
    pub fn get_watermark(&self, key: &K) -> Option<String> {
        let guard = self.watermarks.lock().unwrap_or_else(|e| e.into_inner());
        guard.0.get(key).cloned()
    }
}

/// SQL DDL to create the auxiliary causal watermark table for schemas that lack native HLC columns.
pub const ENSURE_WATERMARK_TABLE_SQL: &str = "\
CREATE TABLE IF NOT EXISTS _flux_causal_watermarks (\
    namespace VARCHAR(64) NOT NULL,\
    entity_id VARCHAR(128) NOT NULL,\
    hlc VARCHAR(64) NOT NULL,\
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),\
    PRIMARY KEY (namespace, entity_id)\
);";

/// Atomically evaluates and advances a causal high-water mark inside a host DB transaction.
///
/// This allows existing legacy database tables that only have `updated_at` timestamps
/// to achieve strict monotonic causal consistency without altering their schema.
pub fn advance_db_watermark(
    tx: &mut Transaction,
    namespace: &str,
    entity_id: &str,
    incoming_hlc: &str,
) -> Result<CausalVerdict, String> {
    // 1. Query current watermark inside transaction
    let query_sql = "SELECT hlc FROM _flux_causal_watermarks WHERE namespace = $1 AND entity_id = $2";
    let rows = tx.query(query_sql, &serde_json::json!([namespace, entity_id]))?;

    if let Some(first) = rows.first() {
        if let Some(existing_hlc) = first.get("hlc").and_then(|v| v.as_str()) {
            let verdict = evaluate_causality(incoming_hlc, existing_hlc);
            if !verdict.is_fresh() {
                return Ok(verdict);
            }
        }
    }

    // 2. Upsert the fresh high-water mark with conditional monotonic conflict protection
    let upsert_sql = "\
    INSERT INTO _flux_causal_watermarks (namespace, entity_id, hlc, updated_at) \
    VALUES ($1, $2, $3, NOW()) \
    ON CONFLICT (namespace, entity_id) DO UPDATE \
    SET hlc = EXCLUDED.hlc, updated_at = NOW() \
    WHERE CASE WHEN _flux_causal_watermarks.hlc LIKE '%.%' AND EXCLUDED.hlc LIKE '%.%' THEN \
        (split_part(_flux_causal_watermarks.hlc, '.', 1)::bigint < split_part(EXCLUDED.hlc, '.', 1)::bigint OR \
        (split_part(_flux_causal_watermarks.hlc, '.', 1)::bigint = split_part(EXCLUDED.hlc, '.', 1)::bigint AND \
        split_part(_flux_causal_watermarks.hlc, '.', 2)::bigint < split_part(EXCLUDED.hlc, '.', 2)::bigint)) \
        ELSE _flux_causal_watermarks.hlc < EXCLUDED.hlc END";

    tx.execute(upsert_sql, &serde_json::json!([namespace, entity_id, incoming_hlc]))?;
    Ok(CausalVerdict::Fresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evaluate_causality_verdicts() {
        assert_eq!(evaluate_causality("100.0002", "100.0001"), CausalVerdict::Fresh);
        assert_eq!(evaluate_causality("101.0001", "100.0009"), CausalVerdict::Fresh);
        assert_eq!(evaluate_causality("100.0001", "100.0001"), CausalVerdict::Duplicate);
        assert_eq!(evaluate_causality("100.0001", "100.0002"), CausalVerdict::Stale);
        assert_eq!(evaluate_causality("99.9999", "100.0001"), CausalVerdict::Stale);
    }

    #[test]
    fn test_causal_guard_advance() {
        let guard = CausalGuard::new(10);
        let key = ("company-a".to_string(), "user-1".to_string());

        assert_eq!(guard.evaluate_and_advance(&key, "100.0001"), CausalVerdict::Fresh);
        assert_eq!(guard.get_watermark(&key), Some("100.0001".to_string()));

        // Duplicate
        assert_eq!(guard.evaluate_and_advance(&key, "100.0001"), CausalVerdict::Duplicate);

        // Stale
        assert_eq!(guard.evaluate_and_advance(&key, "99.9999"), CausalVerdict::Stale);

        // Newer
        assert_eq!(guard.evaluate_and_advance(&key, "100.0002"), CausalVerdict::Fresh);
        assert_eq!(guard.get_watermark(&key), Some("100.0002".to_string()));
    }
}
