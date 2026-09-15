//! Hybrid Logical Clock (HLC) & Monotonic Causality Utilities
//!
//! Provides deterministic causality checks and duplicate event detection
//! based on monotonic HLC timestamps (`<wall_clock_ms>.<logical_counter>`).

/// Parses an HLC string in the format `<wall_ms>.<logical_counter>` (or `<wall_ms>`).
pub fn parse_hlc(hlc_str: &str) -> Option<(u64, u32)> {
    let clean = hlc_str.trim();
    if clean.is_empty() {
        return None;
    }

    if let Some((wall_str, count_str)) = clean.split_once('.') {
        let wall: u64 = wall_str.parse().ok()?;
        let count: u32 = count_str.parse().ok()?;
        Some((wall, count))
    } else {
        let wall: u64 = clean.parse().ok()?;
        Some((wall, 0))
    }
}

/// Returns `true` if the `incoming` HLC is older than or equal to `existing` (i.e. stale or duplicate).
///
/// Under causal monotonic ordering, any incoming event whose HLC does not strictly advance
/// the entity's high-water mark should be discarded to preserve idempotency.
pub fn is_stale(incoming: &str, existing: &str) -> bool {
    let inc = parse_hlc(incoming);
    let ext = parse_hlc(existing);

    match (inc, ext) {
        (Some((w_inc, c_inc)), Some((w_ext, c_ext))) => {
            if w_inc < w_ext {
                true
            } else if w_inc > w_ext {
                false
            } else {
                c_inc <= c_ext
            }
        }
        _ => incoming <= existing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hlc() {
        assert_eq!(parse_hlc("1700000000.000005"), Some((1700000000, 5)));
        assert_eq!(parse_hlc("1700000000"), Some((1700000000, 0)));
        assert_eq!(parse_hlc("invalid"), None);
    }

    #[test]
    fn test_is_stale_monotonicity() {
        assert!(is_stale("100.0001", "100.0001")); // Identical HLC is stale (duplicate)
        assert!(is_stale("100.0001", "100.0002")); // Older count is stale
        assert!(is_stale("99.9999", "100.0001"));  // Older wall time is stale
        assert!(!is_stale("100.0002", "100.0001")); // Newer count is fresh
        assert!(!is_stale("101.0000", "100.0005")); // Newer wall time is fresh
    }
}
