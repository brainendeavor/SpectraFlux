// AssemblyScript SDK: Outbound Broker Host Bridge (`host_broker`)

@external("host_broker", "publish")
declare function host_broker_publish(
  topic_ptr: u32,
  topic_len: u32,
  payload_ptr: u32,
  payload_len: u32
): u32;

/**
 * Publishes an event or message to an outbound topic via the SpectraFlux host broker bridge.
 * @param topic The destination topic name (e.g. "auth.session_created", "orders.processed").
 * @param payloadJson Serialized JSON or UTF-8 message string.
 */
export function publish(topic: string, payloadJson: string): bool {
  const topicBuf = String.UTF8.encode(topic);
  const payloadBuf = String.UTF8.encode(payloadJson);
  const res = host_broker_publish(
    changetype<u32>(topicBuf),
    topicBuf.byteLength as u32,
    changetype<u32>(payloadBuf),
    payloadBuf.byteLength as u32
  );
  return res == 1;
}
