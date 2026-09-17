use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::storage::FluxStorage;

/// Low-level span capturing a synchronous or asynchronous host capability execution
/// during a fluxcell step (e.g. database query, checkpoint save/get, or KV store operation).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostCallSpan {
    pub call_type: String,       // "checkpoint:get", "checkpoint:save", "db:query", "db:execute", "db:commit", "db:rollback", "kv:get", "kv:set", "kv:delete"
    pub target: String,          // Key name or SQL table/statement
    pub duration_us: u64,        // Microseconds elapsed
    pub status: String,          // "ok", "hit", "miss", "error"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Execution span of an individual fluxcell invoked during the mutation event lifecycle.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FluxcellStepSpan {
    pub fluxcell_name: String,
    pub topic: String,
    pub function_name: String,
    pub start_time: String,
    pub duration_ms: f64,
    pub status: String,          // "ok", "error", "dead_letter", "skipped"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_preview: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub host_calls: Vec<HostCallSpan>,
}

fn default_queue_ingress() -> String {
    "QUEUE".to_string()
}

/// End-to-end domain mutation trace scoping input, cross-fluxcell execution DAG,
/// host calls, and terminal output by the gateway's monotonic UUIDv7 command ID and HLC.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainOperationTrace {
    pub command_id: String,      // Monotonic UUIDv7 from SpectraGQL
    pub hlc: String,             // Hybrid Logical Clock timestamp
    pub topic: String,           // Event topic e.g. "mutation.coeval.createInterview" or "http:POST /votes/mutate"
    pub operation_name: String,  // Operation name e.g. "createInterview"
    #[serde(default = "default_queue_ingress")]
    pub ingress: String,         // "HTTP" or "QUEUE"
    pub worker_id: String,       // Worker chassis ID
    pub status: String,          // "completed", "failed", "dlq"
    pub total_duration_ms: f64,
    pub started_at: String,
    pub completed_at: String,
    pub initial_input: serde_json::Value,
    pub steps: Vec<FluxcellStepSpan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_output: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Compact overview summary of a domain trace for rapid tabular indexing.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentTraceSummary {
    pub command_id: String,
    pub hlc: String,
    pub topic: String,
    pub operation_name: String,
    #[serde(default = "default_queue_ingress")]
    pub ingress: String,
    pub status: String,
    pub total_duration_ms: f64,
    pub started_at: String,
    pub step_count: usize,
    pub host_call_count: usize,
}

impl From<&DomainOperationTrace> for RecentTraceSummary {
    fn from(t: &DomainOperationTrace) -> Self {
        let host_call_count = t.steps.iter().map(|s| s.host_calls.len()).sum();
        Self {
            command_id: t.command_id.clone(),
            hlc: t.hlc.clone(),
            topic: t.topic.clone(),
            operation_name: t.operation_name.clone(),
            ingress: t.ingress.clone(),
            status: t.status.clone(),
            total_duration_ms: t.total_duration_ms,
            started_at: t.started_at.clone(),
            step_count: t.steps.len(),
            host_call_count,
        }
    }
}

/// Durable, storage-backed domain trace store using FluxStorage (Kevy or Redis).
pub struct DomainTraceStorage {
    storage: Arc<dyn FluxStorage>,
    in_memory_recent: std::sync::RwLock<std::collections::VecDeque<RecentTraceSummary>>,
    in_memory_traces: std::sync::RwLock<std::collections::HashMap<String, DomainOperationTrace>>,
    max_recent: usize,
}

impl DomainTraceStorage {
    pub fn new(storage: Arc<dyn FluxStorage>) -> Self {
        Self {
            storage,
            in_memory_recent: std::sync::RwLock::new(std::collections::VecDeque::with_capacity(200)),
            in_memory_traces: std::sync::RwLock::new(std::collections::HashMap::with_capacity(200)),
            max_recent: 200,
        }
    }

    pub async fn record_trace(&self, trace: &DomainOperationTrace) -> Result<()> {
        let json_str = serde_json::to_string(trace)?;
        let key = format!("trace:{}", trace.command_id);

        // 24h TTL (86400 seconds) in FluxStorage
        let _ = self.storage.set(&key, &json_str, 86400).await;

        let summary = RecentTraceSummary::from(trace);

        // Update in-memory fast lookup cache
        {
            let mut recent = self.in_memory_recent.write().unwrap();
            recent.retain(|r| r.command_id != trace.command_id);
            recent.push_front(summary.clone());
            while recent.len() > self.max_recent {
                let popped = recent.pop_back();
                if let Some(p) = popped {
                    let mut traces = self.in_memory_traces.write().unwrap();
                    traces.remove(&p.command_id);
                }
            }
            let mut traces = self.in_memory_traces.write().unwrap();
            traces.insert(trace.command_id.clone(), trace.clone());
        }

        // Persist recent index to FluxStorage
        let recent_vec: Vec<RecentTraceSummary> = {
            self.in_memory_recent.read().unwrap().iter().cloned().collect()
        };
        if let Ok(idx_json) = serde_json::to_string(&recent_vec) {
            let _ = self.storage.set("traces:recent_index", &idx_json, 86400).await;
        }

        Ok(())
    }

    pub async fn get_trace(&self, command_id: &str) -> Result<Option<DomainOperationTrace>> {
        // Fast path: in-memory cache
        {
            let traces = self.in_memory_traces.read().unwrap();
            if let Some(t) = traces.get(command_id) {
                return Ok(Some(t.clone()));
            }
        }

        // Fallback: load from FluxStorage
        let key = format!("trace:{}", command_id);
        if let Some(raw_json) = self.storage.get(&key).await? {
            if let Ok(trace) = serde_json::from_str::<DomainOperationTrace>(&raw_json) {
                let mut traces = self.in_memory_traces.write().unwrap();
                traces.insert(command_id.to_string(), trace.clone());
                return Ok(Some(trace));
            }
        }

        Ok(None)
    }

    pub async fn list_recent_traces(&self, limit: usize) -> Result<Vec<RecentTraceSummary>> {
        // Fast path: in-memory list if populated
        let in_mem: Vec<RecentTraceSummary> = {
            let recent = self.in_memory_recent.read().unwrap();
            recent.iter().take(limit).cloned().collect()
        };
        if !in_mem.is_empty() {
            return Ok(in_mem);
        }

        // Fallback: load index from storage
        if let Some(raw_json) = self.storage.get("traces:recent_index").await? {
            if let Ok(list) = serde_json::from_str::<Vec<RecentTraceSummary>>(&raw_json) {
                let mut recent = self.in_memory_recent.write().unwrap();
                *recent = list.clone().into();
                return Ok(list.into_iter().take(limit).collect());
            }
        }

        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::KevyStorage;

    #[tokio::test]
    async fn test_domain_trace_storage_record_and_get() {
        let storage = Arc::new(KevyStorage::new_in_memory().unwrap());
        let trace_store = DomainTraceStorage::new(storage);

        let trace = DomainOperationTrace {
            command_id: "018f3a2b-7c4d-7a8e-9012-3456789abcde".to_string(),
            hlc: "1710600000000:1".to_string(),
            topic: "mutation.coeval.createInterview".to_string(),
            operation_name: "createInterview".to_string(),
            ingress: "QUEUE".to_string(),
            worker_id: "spectral-flux-worker-1".to_string(),
            status: "completed".to_string(),
            total_duration_ms: 12.45,
            started_at: "2026-09-16T12:00:00Z".to_string(),
            completed_at: "2026-09-16T12:00:00.012Z".to_string(),
            initial_input: serde_json::json!({ "candidateId": "cand-42", "jobId": "eng-101" }),
            steps: vec![
                FluxcellStepSpan {
                    fluxcell_name: "interview-orchestrator".to_string(),
                    topic: "mutation.coeval.createInterview".to_string(),
                    function_name: "handle_event".to_string(),
                    start_time: "2026-09-16T12:00:00.001Z".to_string(),
                    duration_ms: 8.2,
                    status: "ok".to_string(),
                    input_preview: Some(serde_json::json!({ "candidateId": "cand-42" })),
                    output_preview: Some(serde_json::json!({ "interviewId": "int-99" })),
                    error: None,
                    host_calls: vec![
                        HostCallSpan {
                            call_type: "db:query".to_string(),
                            target: "SELECT * FROM candidates WHERE id = $1".to_string(),
                            duration_us: 1250,
                            status: "ok".to_string(),
                            detail: Some("rows: 1".to_string()),
                        },
                        HostCallSpan {
                            call_type: "checkpoint:save".to_string(),
                            target: "chk:018f3a2b-7c4d-7a8e-9012-3456789abcde:reserve_slot".to_string(),
                            duration_us: 340,
                            status: "ok".to_string(),
                            detail: Some("ttl: 86400s".to_string()),
                        },
                    ],
                }
            ],
            terminal_output: Some(serde_json::json!({ "status": "scheduled", "interviewId": "int-99" })),
            error: None,
        };

        trace_store.record_trace(&trace).await.unwrap();

        // Retrieve full trace
        let fetched = trace_store.get_trace("018f3a2b-7c4d-7a8e-9012-3456789abcde").await.unwrap().expect("trace should exist");
        assert_eq!(fetched.command_id, trace.command_id);
        assert_eq!(fetched.operation_name, "createInterview");
        assert_eq!(fetched.steps.len(), 1);
        assert_eq!(fetched.steps[0].host_calls.len(), 2);
        assert_eq!(fetched.steps[0].host_calls[0].call_type, "db:query");

        // Retrieve summary list
        let summaries = trace_store.list_recent_traces(10).await.unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].command_id, trace.command_id);
        assert_eq!(summaries[0].step_count, 1);
        assert_eq!(summaries[0].host_call_count, 2);
    }
}
