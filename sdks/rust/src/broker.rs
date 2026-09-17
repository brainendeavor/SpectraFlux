//! Outbound Broker Dispatching Bridge (`host_broker`)

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "host_broker")]
unsafe extern "C" {
    safe fn publish(topic_ptr: u32, topic_len: u32, payload_ptr: u32, payload_len: u32) -> u32;
}

/// Publishes an event to the host broker on the given topic.
pub fn publish_event(topic: &str, payload_json: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let t_bytes = topic.as_bytes();
        let p_bytes = payload_json.as_bytes();
        let res = publish(
            t_bytes.as_ptr() as usize as u32,
            t_bytes.len() as u32,
            p_bytes.as_ptr() as usize as u32,
            p_bytes.len() as u32,
        );
        res == 1
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (topic, payload_json);
        true
    }
}
