# TypeScript Starter Seed (`fluxcell-seed-ts`)

Canonical barebones TypeScript starter seed for building WebAssembly **Fluxcells** on **SpectraFlux** using AssemblyScript and `@spectraflux/sdk`.

---

## 1. Quick Start (Bun-First)

```bash
# 1. Install dependencies
bun install

# 2. Compile to WebAssembly (< 25 KB release artifact)
bun run build

# 3. Test and verify Wasmtime ABI compatibility
bun test

# 4. Deploy directly to downstream SpectraFlux chassis (Dev Upload)
bun run deploy --dev-upload --mount /api/starter
```

*(Note: If Bun is not installed, you can use `npm install && npm run build && npm test`)*

---

## 2. Project Layout

* **`assembly/index.ts`**: Main Fluxcell logic. Implements `StarterFluxcell` extending `Fluxcell`.
* **`wit/fluxcell.wit`**: Canonical SpectraFlux WIT interface specification.
* **`asconfig.json`**: AssemblyScript build configuration for release and debug targets.
* **`build.sh`**: Standalone shell build runner computing SHA-256 digests.

---

## 3. Writing Your Fluxcell

```typescript
import {
  Fluxcell,
  FluxcellMetadata,
  HttpRequest,
  HttpResponse,
  RouteMeta,
  registerFluxcell,
} from "@spectraflux/sdk";

export class MyCustomFluxcell extends Fluxcell {
  metadata(): FluxcellMetadata {
    return new FluxcellMetadata("0.1.0", "My Custom Fluxcell");
  }

  routes(): RouteMeta[] {
    return [
      RouteMeta.get("/health", "Health check"),
      RouteMeta.post("/mutate", "Execute transactional mutation"),
    ];
  }

  handleHttp(req: HttpRequest): HttpResponse {
    if (req.path == "/health") {
      return HttpResponse.json('{"status":"ok"}');
    }
    return HttpResponse.notFound("Not found");
  }
}

registerFluxcell(new MyCustomFluxcell());

// Re-export C-ABI functions for the SpectraFlux Wasmtime host
export {
  allocate,
  deallocate,
  get_metadata,
  get_subscriptions,
  get_routes,
  handle_http,
  handle_event,
} from "@spectraflux/sdk";
```

---

## 4. Compilation & Deployment

* **Release Build:** `bun run build` generates `build/release.wasm` and `build/release.wasm.sha256`.
* **Direct Dev Upload:** `bun run deploy --dev-upload --mount /api/my-cell --activate`
* **Status Check:** `bun run status`
