import { test, expect } from "bun:test";
import fs from "node:fs";
import path from "node:path";

test("SpectraFlux WASM ABI Verification", async () => {
  const wasmPath = path.resolve(import.meta.dir, "../build/release.wasm");
  expect(fs.existsSync(wasmPath)).toBe(true);

  const bytes = fs.readFileSync(wasmPath);
  // Zero-bloat invariant: sub-50 KB binary
  expect(bytes.length).toBeLessThan(50 * 1024);

  const mockDb = {
    begin_tx: () => 0n,
    execute: () => 0n,
    query: () => 0n,
    commit_tx: () => 0n,
    rollback_tx: () => 0n,
  };

  const importObject = {
    host_db: mockDb,
    env: { abort: () => {} },
  };

  const { instance } = await WebAssembly.instantiate(bytes, importObject);
  const exports = instance.exports as any;

  // 1. Core memory & allocator
  expect(typeof exports.memory).toBe("object");
  expect(typeof exports.allocate).toBe("function");
  expect(typeof exports.deallocate).toBe("function");

  // 2. Metadata, routes, subscriptions
  expect(typeof exports.get_metadata).toBe("function");
  expect(typeof exports.get_routes).toBe("function");
  expect(typeof exports.get_subscriptions).toBe("function");

  // 3. Execution handlers
  expect(typeof exports.handle_http).toBe("function");
  expect(typeof exports.handle_event).toBe("function");

  // Helper to read packed guest strings
  function readGuestString(packed: bigint | number): string {
    const p = BigInt(packed);
    const ptr = Number(p >> 32n);
    const len = Number(p & 0xffffffffn);
    const view = new Uint8Array(exports.memory.buffer, ptr, len);
    return Buffer.from(view).toString("utf8");
  }

  function writeGuestString(str: string): { ptr: number; len: number } {
    const b = Buffer.from(str, "utf8");
    const ptr = exports.allocate(b.length);
    const view = new Uint8Array(exports.memory.buffer, ptr, b.length);
    view.set(b);
    return { ptr, len: b.length };
  }

  // Verify get_metadata
  const metaStr = readGuestString(exports.get_metadata());
  const meta = JSON.parse(metaStr);
  expect(meta.version).toBe("0.1.0");

  // Verify get_routes
  const routesStr = readGuestString(exports.get_routes());
  const routes = JSON.parse(routesStr);
  expect(Array.isArray(routes)).toBe(true);
  expect(routes.length).toBeGreaterThanOrEqual(2);

  // Verify get_subscriptions
  const subsStr = readGuestString(exports.get_subscriptions());
  const subs = JSON.parse(subsStr);
  expect(Array.isArray(subs)).toBe(true);
  expect(subs).toContain("events.sample");

  // Verify handle_http /health
  const req1 = writeGuestString(JSON.stringify({ path: "/health", method: "GET", headers: [], body: "" }));
  const resp1Packed = exports.handle_http(req1.ptr, req1.len);
  exports.deallocate(req1.ptr, req1.len);
  const resp1 = JSON.parse(readGuestString(resp1Packed));
  expect(resp1.status).toBe(200);
  const stats = JSON.parse(resp1.body);
  expect(stats.status).toBe("healthy");
  expect(stats.total_processed).toBeGreaterThanOrEqual(1);

  // Verify handle_event with fresh HLC 100.001
  const ev1 = writeGuestString(JSON.stringify({
    event_id: "ev-1",
    hlc: "100.001",
    topic: "events.sample",
    payload: "{}",
  }));
  const evResp1 = JSON.parse(readGuestString(exports.handle_event(ev1.ptr, ev1.len)));
  exports.deallocate(ev1.ptr, ev1.len);
  expect(evResp1.status).toBe("processed");

  // Verify duplicate HLC 100.001 is discarded by CausalGuard
  const evDup = writeGuestString(JSON.stringify({
    event_id: "ev-dup",
    hlc: "100.001",
    topic: "events.sample",
    payload: "{}",
  }));
  const evRespDup = JSON.parse(readGuestString(exports.handle_event(evDup.ptr, evDup.len)));
  exports.deallocate(evDup.ptr, evDup.len);
  expect(evRespDup.status).toBe("discarded");
  expect(evRespDup.reason).toBe("stale_or_duplicate_hlc");

  // Verify stale older HLC 99.999 is discarded
  const evStale = writeGuestString(JSON.stringify({
    event_id: "ev-stale",
    hlc: "99.999",
    topic: "events.sample",
    payload: "{}",
  }));
  const evRespStale = JSON.parse(readGuestString(exports.handle_event(evStale.ptr, evStale.len)));
  exports.deallocate(evStale.ptr, evStale.len);
  expect(evRespStale.status).toBe("discarded");

  // Verify newer HLC 100.002 is accepted
  const evNew = writeGuestString(JSON.stringify({
    event_id: "ev-new",
    hlc: "100.002",
    topic: "events.sample",
    payload: "{}",
  }));
  const evRespNew = JSON.parse(readGuestString(exports.handle_event(evNew.ptr, evNew.len)));
  exports.deallocate(evNew.ptr, evNew.len);
  expect(evRespNew.status).toBe("processed");
});
