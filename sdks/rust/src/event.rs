//! Asynchronous Event Processing & Verdict Envelopes
//!
//! Provides parsing and verdict encoding for broker event streams
//! dispatched by the SpectraFlux chassis.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Context of an incoming broker message dispatched to the Fluxcell.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventContext {
    pub event_id: String,
    pub topic: String,
    pub hlc: String,
    pub payload: Vec<u8>,
}

impl EventContext {
    /// Decodes an `EventContext` from raw payload bytes or structured JSON.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(bytes) {
            let event_id = val.pointer("/id")
                .or_else(|| val.pointer("/event_id"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            let topic = val.pointer("/topic")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            let hlc = val.pointer("/hlc")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            Self {
                event_id,
                topic,
                hlc,
                payload: bytes.to_vec(),
            }
        } else {
            Self {
                event_id: String::new(),
                topic: String::new(),
                hlc: String::new(),
                payload: bytes.to_vec(),
            }
        }
    }

    /// Deserializes the event payload as JSON into `T`.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.payload)
    }

    /// Executes a durable, idempotent step.
    /// If the step has already executed for this command/event ID,
    /// returns the cached result immediately without invoking `action`.
    pub fn step<F, T>(&self, step_name: &str, action: F) -> Result<T, String>
    where
        F: FnOnce() -> Result<T, String>,
        T: Serialize + DeserializeOwned,
    {
        if let Some(cached_json) = checkpoint::get_step(step_name) {
            if let Ok(val) = serde_json::from_str::<T>(&cached_json) {
                return Ok(val);
            }
        }

        let res = action()?;
        if let Ok(json_str) = serde_json::to_string(&res) {
            checkpoint::save_step(step_name, &json_str, 86_400);
        }
        Ok(res)
    }
}

pub mod checkpoint {
    #[cfg(target_arch = "wasm32")]
    #[link(wasm_import_module = "checkpoint")]
    unsafe extern "C" {
        safe fn get(step_ptr: u32, step_len: u32) -> u64;
        safe fn save(step_ptr: u32, step_len: u32, val_ptr: u32, val_len: u32, ttl_seconds: u64) -> u32;
    }

    /// Retrieves cached step result if present
    pub fn get_step(step_name: &str) -> Option<String> {
        #[cfg(target_arch = "wasm32")]
        {
            let bytes = step_name.as_bytes();
            let packed = get(bytes.as_ptr() as usize as u32, bytes.len() as u32);
            if packed == 0 {
                return None;
            }
            let ptr = (packed >> 32) as usize;
            let len = (packed & 0xFFFF_FFFF) as usize;
            let slice = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
            let res = String::from_utf8(slice.to_vec()).ok();
            crate::abi::deallocate(ptr as *mut u8, len);
            res
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = step_name;
            None
        }
    }

    /// Saves step result with optional TTL in seconds
    pub fn save_step(step_name: &str, result_json: &str, ttl_seconds: u64) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            let s_bytes = step_name.as_bytes();
            let r_bytes = result_json.as_bytes();
            let res = save(
                s_bytes.as_ptr() as usize as u32,
                s_bytes.len() as u32,
                r_bytes.as_ptr() as usize as u32,
                r_bytes.len() as u32,
                ttl_seconds,
            );
            res != 0
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (step_name, result_json, ttl_seconds);
            true
        }
    }
}

/// The outcome verdict returned after processing an event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EventVerdict {
    Ack,
    Nack(String),
    DeadLetter(String),
    IgnoredStaleHlc,
}

impl EventVerdict {
    /// Serializes the verdict into a standard JSON envelope matching the chassis expectation.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Ack => serde_json::json!({ "status": "ok" }),
            Self::Nack(err) => serde_json::json!({ "status": "nack", "error": err }),
            Self::DeadLetter(err) => serde_json::json!({ "status": "dead_letter", "error": err }),
            Self::IgnoredStaleHlc => serde_json::json!({ "status": "IGNORED_STALE_HLC" }),
        }
    }
}
