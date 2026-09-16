// AssemblyScript SDK: Low-Level C-ABI & Wasmtime Host Bridge

import { HttpRequest, HttpResponse, RouteMeta } from "./http";
import { EventContext } from "./event";
import { FluxcellMetadata } from "./metadata";
import { step } from "./checkpoint";


export class Fluxcell {
  metadata(): FluxcellMetadata {
    return new FluxcellMetadata("0.1.0", "SpectraFlux Fluxcell");
  }

  routes(): RouteMeta[] {
    return [];
  }

  subscriptions(): string[] {
    return [];
  }

  handleHttp(req: HttpRequest): HttpResponse {
    return HttpResponse.notFound("Endpoint not handled by this fluxcell");
  }

  handleEvent(event: EventContext): string {
    return '{"status":"ok"}';
  }
}

let activeCell: Fluxcell | null = null;

export function registerFluxcell(cell: Fluxcell): void {
  activeCell = cell;
}

export function getActiveFluxcell(): Fluxcell {
  if (activeCell == null) {
    activeCell = new Fluxcell();
  }
  return activeCell!;
}

// Low-Level WASM Memory Management
export function allocate(size: u32): usize {
  return heap.alloc(size as usize);
}

export function deallocate(ptr: usize, size: u32): void {
  heap.free(ptr);
}

export function packString(str: string): u64 {
  const buf = String.UTF8.encode(str);
  const len: usize = buf.byteLength;
  const ptr: usize = heap.alloc(len);
  memory.copy(ptr, changetype<usize>(buf), len);
  return (u64(ptr) << 32) | u64(len);
}

export function unpackString(ptr: u32, len: u32): string {
  if (len == 0) return "";
  return String.UTF8.decodeUnsafe(ptr as usize, len as usize);
}

// C-ABI Exports for SpectraFlux Wasmtime Host
export function get_metadata(): u64 {
  const cell = getActiveFluxcell();
  const meta = cell.metadata();
  return packString(meta.toJson());
}

export function get_subscriptions(): u64 {
  const cell = getActiveFluxcell();
  const subs = cell.subscriptions();
  let json = "[";
  for (let i = 0; i < subs.length; i++) {
    if (i > 0) json += ",";
    json += '"' + subs[i] + '"';
  }
  json += "]";
  return packString(json);
}

export function get_routes(): u64 {
  const cell = getActiveFluxcell();
  const routes = cell.routes();
  let json = "[";
  for (let i = 0; i < routes.length; i++) {
    if (i > 0) json += ",";
    json += routes[i].toJson();
  }
  json += "]";
  return packString(json);
}

export function handle_http(ptr: u32, len: u32): u64 {
  const cell = getActiveFluxcell();
  const reqJson = unpackString(ptr, len);
  const req = HttpRequest.fromJson(reqJson);
  const resp = cell.handleHttp(req);
  return packString(resp.toJson());
}

export function handle_event(ptr: u32, len: u32): u64 {
  const cell = getActiveFluxcell();
  const eventJson = unpackString(ptr, len);
  const event = EventContext.fromJson(eventJson);
  const verdict = cell.handleEvent(event);
  return packString(verdict);
}

let testInvoked: i32 = 0;
let testReturnVal: string = "";

function stepActionOne(): string {
  testInvoked++;
  return testReturnVal;
}

function stepActionTwo(): string {
  testInvoked++;
  return "unexpected";
}

// Verification export for step checkpoint memoization
export function test_checkpoint_step(stepPtr: u32, stepLen: u32, valPtr: u32, valLen: u32): u64 {
  const stepName = unpackString(stepPtr, stepLen);
  testReturnVal = unpackString(valPtr, valLen);
  testInvoked = 0;

  const res = step(stepName, stepActionOne);
  const res2 = step(stepName, stepActionTwo);

  const out = '{"result":' + res + ',"cached":' + res2 + ',"invocations":' + testInvoked.toString() + '}';
  return packString(out);
}


