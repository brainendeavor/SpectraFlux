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
