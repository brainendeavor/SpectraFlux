import { test, expect } from "bun:test";
import fs from "node:fs";
import path from "node:path";

test("SpectraFlux TypeScript SDK ABI & Runtime Verification", async () => {
  const wasmPath = path.resolve(import.meta.dir, "../build/release.wasm");
  expect(fs.existsSync(wasmPath)).toBe(true);

  const bytes = fs.readFileSync(wasmPath);
  // Zero-bloat invariant: sub-50 KB binary
  expect(bytes.length).toBeLessThan(50 * 1024);

  let txIdCounter = 1000n;
  const mockDb = {
    begin_tx: () => txIdCounter++,
    execute: () => 1n,
    query: () => 0n,
    commit_tx: () => 0n,
    rollback_tx: () => 0n,
  };

  let exportsRef: any = null;
  const checkpointMap = new Map<string, string>();
  const mockCheckpoint = {
    get: (stepPtr: number, stepLen: number): bigint => {
      if (!exportsRef) return 0n;
      const view = new Uint8Array(exportsRef.memory.buffer, stepPtr, stepLen);
      const name = Buffer.from(view).toString("utf8");
      if (checkpointMap.has(name)) {
        const val = checkpointMap.get(name)!;
        const b = Buffer.from(val, "utf8");
        const ptr = exportsRef.allocate(b.length);
        const v = new Uint8Array(exportsRef.memory.buffer, ptr, b.length);
        v.set(b);
        return (BigInt(ptr) << 32n) | BigInt(b.length);
      }
      return 0n;
    },
    save: (stepPtr: number, stepLen: number, valPtr: number, valLen: number, ttl: bigint): number => {
      if (!exportsRef) return 0;
      const stepView = new Uint8Array(exportsRef.memory.buffer, stepPtr, stepLen);
      const name = Buffer.from(stepView).toString("utf8");
      const valView = new Uint8Array(exportsRef.memory.buffer, valPtr, valLen);
      const val = Buffer.from(valView).toString("utf8");
      checkpointMap.set(name, val);
      return 1;
    },
  };

  const importObject = {
    host_db: mockDb,
    checkpoint: mockCheckpoint,
    env: {
      abort: (msg: any, file: any, line: any, col: any) => {
        console.error(`Abort called: ${file}:${line}:${col}`);
      },
    },
  };

  const { instance } = await WebAssembly.instantiate(bytes, importObject);
  const exports = instance.exports as any;
  exportsRef = exports;


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

  // Verify get_subscriptions
  const subsStr = readGuestString(exports.get_subscriptions());
  const subs = JSON.parse(subsStr);
  expect(Array.isArray(subs)).toBe(true);

  // Verify handle_http default response
  const req = writeGuestString(
    JSON.stringify({ path: "/unknown", method: "GET", headers: [], body: "" })
  );
  const respPacked = exports.handle_http(req.ptr, req.len);
  exports.deallocate(req.ptr, req.len);
  const resp = JSON.parse(readGuestString(respPacked));
  expect(resp.status).toBe(404);

  // Verify handle_event default response
  const ev = writeGuestString(
    JSON.stringify({
      event_id: "evt-01",
      hlc: "100-1",
      topic: "mutation.test",
      payload: "{}",
    })
  );
  const evRespPacked = exports.handle_event(ev.ptr, ev.len);
  exports.deallocate(ev.ptr, ev.len);
  const evResp = JSON.parse(readGuestString(evRespPacked));
  expect(evResp.status).toBe("ok");

  // 4. Verify Checkpoint Step Memoization & Deduplication
  expect(typeof exports.test_checkpoint_step).toBe("function");

  const chkStep = writeGuestString("reserve_inventory");
  const chkVal = writeGuestString('{"reserved":true,"sku":"item-999"}');
  const chkResultPacked = exports.test_checkpoint_step(chkStep.ptr, chkStep.len, chkVal.ptr, chkVal.len);
  exports.deallocate(chkStep.ptr, chkStep.len);
  exports.deallocate(chkVal.ptr, chkVal.len);

  const chkResult = JSON.parse(readGuestString(chkResultPacked));
  expect(chkResult.result).toEqual({ reserved: true, sku: "item-999" });
  expect(chkResult.cached).toEqual({ reserved: true, sku: "item-999" });
  expect(chkResult.invocations).toBe(1); // Action closure only invoked ONCE!
  expect(checkpointMap.get("reserve_inventory")).toBe('{"reserved":true,"sku":"item-999"}');
});

