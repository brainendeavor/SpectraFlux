// AssemblyScript SDK: 80/20 Durable Step Checkpoints Host Bridge (`checkpoint`)

@external("checkpoint", "get")
declare function host_checkpoint_get(step_ptr: u32, step_len: u32): u64;

@external("checkpoint", "save")
declare function host_checkpoint_save(
  step_ptr: u32,
  step_len: u32,
  val_ptr: u32,
  val_len: u32,
  ttl_seconds: u64
): u32;

function unpackString(packed: u64): string | null {
  if (packed == 0) return null;
  const ptr = (packed >> 32) as usize;
  const len = (packed & 0xffffffff) as usize;
  if (len == 0) return "";
  const str = String.UTF8.decodeUnsafe(ptr, len);
  heap.free(ptr);
  return str;
}

/**
 * Retrieves the cached step result for the current event/command if previously saved.
 * Returns `null` if no checkpoint exists.
 */
export function getStep(stepName: string): string | null {
  const nameBuf = String.UTF8.encode(stepName);
  const packed = host_checkpoint_get(
    changetype<u32>(nameBuf),
    nameBuf.byteLength as u32
  );
  return unpackString(packed);
}

/**
 * Saves a step result in durable host storage scoped by the current command ID.
 * @param stepName Unique name of the step within the mutation handler.
 * @param resultJson Serialized JSON or string output of the step.
 * @param ttlSeconds TTL in seconds before expiration (default: 86400 / 24 hours).
 */
export function saveStep(
  stepName: string,
  resultJson: string,
  ttlSeconds: u64 = 86400
): bool {
  const nameBuf = String.UTF8.encode(stepName);
  const valBuf = String.UTF8.encode(resultJson);
  const res = host_checkpoint_save(
    changetype<u32>(nameBuf),
    nameBuf.byteLength as u32,
    changetype<u32>(valBuf),
    valBuf.byteLength as u32,
    ttlSeconds
  );
  return res == 1;
}

/**
 * Executes a durable, idempotent step.
 * If the step has already executed for this command/event ID,
 * returns the cached result immediately without invoking `action`.
 *
 * @param stepName Unique name of the step (e.g. "stripe_charge", "inventory_reserve").
 * @param action Callback closure executed only on initial execution.
 * @param ttlSeconds Checkpoint TTL in seconds (default: 86400 / 24 hours).
 */
export function step(
  stepName: string,
  action: () => string,
  ttlSeconds: u64 = 86400
): string {
  const cached = getStep(stepName);
  if (cached != null) {
    return cached;
  }
  const result = action();
  saveStep(stepName, result, ttlSeconds);
  return result;
}
