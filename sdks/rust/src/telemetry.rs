//! In-Guest Telemetry & Log Ring Buffer
//!
//! Provides lightweight, lock-free metrics counters and a bounded ring buffer
//! of recent log entries for downstream consumer workers and Fluxcells.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// A single log entry formatted for the SpectraGQL Admin UI / SWTP.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryLogEntry {
    pub timestamp: String,
    pub level: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hlc: Option<String>,
}

/// Thread-safe in-memory buffer tracking basic throughput counters and recent log rollups.
#[derive(Debug)]
pub struct TelemetryBuffer {
    processed_count: AtomicU64,
    error_count: AtomicU64,
    stale_count: AtomicU64,
    capacity: usize,
    logs: Mutex<VecDeque<TelemetryLogEntry>>,
}

impl TelemetryBuffer {
    /// Creates a new telemetry buffer with the given log capacity (e.g. 100 or 200).
    pub const fn new(capacity: usize) -> Self {
        Self {
            processed_count: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
            stale_count: AtomicU64::new(0),
            capacity,
            logs: Mutex::new(VecDeque::new()),
        }
    }

    /// Increments total processed events/requests.
    pub fn record_processed(&self) {
        self.processed_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments error / poison pill count.
    pub fn record_error(&self) {
        self.error_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments stale HLC / duplicate suppression count.
    pub fn record_stale(&self) {
        self.stale_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Appends a log line to the ring buffer, evicting the oldest entry if capacity is exceeded.
    pub fn log(&self, level: &str, message: impl Into<String>, hlc: Option<&str>) {
        if let Ok(mut logs) = self.logs.lock() {
            if logs.len() >= self.capacity {
                logs.pop_front();
            }
            logs.push_back(TelemetryLogEntry {
                timestamp: String::new(),
                level: level.to_string(),
                message: message.into(),
                hlc: hlc.map(|s| s.to_string()),
            });
        }
    }

    pub fn processed(&self) -> u64 {
        self.processed_count.load(Ordering::Relaxed)
    }

    pub fn errors(&self) -> u64 {
        self.error_count.load(Ordering::Relaxed)
    }

    pub fn stale(&self) -> u64 {
        self.stale_count.load(Ordering::Relaxed)
    }

    /// Returns a JSON value matching the standard health / telemetry schema.
    pub fn stats_json(&self, name: &str, version: &str) -> serde_json::Value {
        serde_json::json!({
            "status": "healthy",
            "fluxcell": name,
            "version": version,
            "total_processed": self.processed(),
            "poison_pills_rejected": self.errors(),
            "stale_hlc_suppressed": self.stale(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telemetry_buffer_counters_and_rollover() {
        let buf = TelemetryBuffer::new(2);
        buf.record_processed();
        buf.record_processed();
        buf.record_error();
        buf.record_stale();

        assert_eq!(buf.processed(), 2);
        assert_eq!(buf.errors(), 1);
        assert_eq!(buf.stale(), 1);

        buf.log("INFO", "msg 1", None);
        buf.log("WARN", "msg 2", None);
        buf.log("ERROR", "msg 3", None);

        let logs = buf.logs.lock().unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].message, "msg 2");
        assert_eq!(logs[1].message, "msg 3");
    }
}
