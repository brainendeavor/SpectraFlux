#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";

async function main() {
  const args = process.argv.slice(2);
  let wasmPath = args[0] || path.resolve(process.cwd(), "build/release.wasm");
  if (!fs.existsSync(wasmPath)) {
    wasmPath = path.resolve(process.cwd(), "build/debug.wasm");
  }

  if (!fs.existsSync(wasmPath)) {
    console.error(`❌ Error: No WASM file found at ${wasmPath}. Run 'bun run build' first.`);
    process.exit(1);
  }

  console.log(`⚡ Testing SpectraFlux WebAssembly ABI for: ${wasmPath}...`);
  const wasmBytes = fs.readFileSync(wasmPath);

  // Mock host_db imports to simulate SpectraFlux host capabilities
  let nextTxId = 100n;
  const mockDb = {
    begin_tx: (namePtr, nameLen) => {
      const tx = nextTxId++;
      return packHostString(`{"ok":${tx}}`);
    },
    execute: (txId, sqlPtr, sqlLen, paramsPtr, paramsLen) => {
      return packHostString(`{"ok":1}`);
    },
    query: (txId, sqlPtr, sqlLen, paramsPtr, paramsLen) => {
      return packHostString(`{"ok":"[]"}`);
    },
    commit_tx: (txId) => {
      return packHostString(`{"ok":null}`);
    },
    rollback_tx: (txId) => {
      return packHostString(`{"ok":null}`);
    },
  };

  function packHostString(s) {
    const bytes = Buffer.from(s, "utf8");
    const len = BigInt(bytes.length);
    // Return dummy packed 64-bit value
    return (1000n << 32n) | (len & 0xffffffffn);
  }

  const mockCheckpoint = {
    get: () => 0n,
    save: () => 1,
  };

  const mockBroker = {
    publish: () => 1,
  };

  const importObject = {
    host_db: mockDb,
    checkpoint: mockCheckpoint,
    host_broker: mockBroker,
    env: {
      abort: (msg, file, line, col) => {
        console.error(`Guest aborted at ${file}:${line}:${col}`);
      },
    },
  };

  const { instance } = await WebAssembly.instantiate(wasmBytes, importObject);
  const exports = instance.exports;

  // 1. Verify Memory
  if (!exports.memory) {
    throw new Error("Missing 'memory' export!");
  }
  console.log("  ✓ [ABI] Exports 'memory'");

  // 2. Verify Allocator
  if (typeof exports.allocate !== "function" || typeof exports.deallocate !== "function") {
    throw new Error("Missing 'allocate' or 'deallocate' export!");
  }
  console.log("  ✓ [ABI] Exports 'allocate' and 'deallocate'");

  const mem = new Uint8Array(exports.memory.buffer);

  function readGuestString(packed) {
    const p = BigInt(packed);
    const ptr = Number(p >> 32n);
    const len = Number(p & 0xffffffffn);
    const view = new Uint8Array(exports.memory.buffer, ptr, len);
    return Buffer.from(view).toString("utf8");
  }

  function writeGuestString(str) {
    const bytes = Buffer.from(str, "utf8");
    const ptr = exports.allocate(bytes.length);
    const view = new Uint8Array(exports.memory.buffer, ptr, bytes.length);
    view.set(bytes);
    return { ptr, len: bytes.length };
  }

  // 3. Verify get_metadata
  if (typeof exports.get_metadata !== "function") {
    throw new Error("Missing 'get_metadata' export!");
  }
  const metaPacked = exports.get_metadata();
  const metaStr = readGuestString(metaPacked);
  const meta = JSON.parse(metaStr);
  console.log(`  ✓ [ABI] 'get_metadata()' returned valid JSON: v${meta.version} (${meta.description || "No desc"})`);

  // 4. Verify get_subscriptions
  if (typeof exports.get_subscriptions !== "function") {
    throw new Error("Missing 'get_subscriptions' export!");
  }
  const subsPacked = exports.get_subscriptions();
  const subsStr = readGuestString(subsPacked);
  const subs = JSON.parse(subsStr);
  if (!Array.isArray(subs)) throw new Error("get_subscriptions must return an array");
  console.log(`  ✓ [ABI] 'get_subscriptions()' returned ${subs.length} topics: [${subs.join(", ")}]`);

  // 5. Verify get_routes
  if (typeof exports.get_routes !== "function") {
    throw new Error("Missing 'get_routes' export!");
  }
  const routesPacked = exports.get_routes();
  const routesStr = readGuestString(routesPacked);
  const routes = JSON.parse(routesStr);
  if (!Array.isArray(routes)) throw new Error("get_routes must return an array");
  console.log(`  ✓ [ABI] 'get_routes()' returned ${routes.length} routes:`);
  for (const r of routes) {
    console.log(`      • ${r.method} ${r.path} (${r.description || ""})`);
  }

  // 6. Verify handle_http
  if (typeof exports.handle_http !== "function") {
    throw new Error("Missing 'handle_http' export!");
  }
  const testReq = JSON.stringify({
    path: routes[0] ? routes[0].path : "/health",
    method: routes[0] ? routes[0].method : "GET",
    headers: [["accept", "application/json"]],
    body: "",
  });
  const reqInput = writeGuestString(testReq);
  const httpPacked = exports.handle_http(reqInput.ptr, reqInput.len);
  exports.deallocate(reqInput.ptr, reqInput.len);

  const httpRespStr = readGuestString(httpPacked);
  const httpResp = JSON.parse(httpRespStr);
  console.log(`  ✓ [ABI] 'handle_http()' returned status ${httpResp.status}: ${httpRespStr.substring(0, 80)}...`);

  // 7. Verify handle_event
  if (typeof exports.handle_event !== "function") {
    throw new Error("Missing 'handle_event' export!");
  }
  const testEvent = JSON.stringify({
    event_id: "test-event-001",
    hlc: "0-0",
    topic: subs[0] || "events.incoming",
    payload_json: '{"test":true}',
  });
  const eventInput = writeGuestString(testEvent);
  const eventPacked = exports.handle_event(eventInput.ptr, eventInput.len);
  exports.deallocate(eventInput.ptr, eventInput.len);

  const eventRespStr = readGuestString(eventPacked);
  console.log(`  ✓ [ABI] 'handle_event()' processed event: ${eventRespStr}`);

  console.log("\n✅ ALL SPECTRAFLUX ABI CHECKS PASSED!");
}

main().catch((err) => {
  console.error("❌ ABI Verification Failed:", err);
  process.exit(1);
});
