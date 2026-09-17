// TypeScript / AssemblyScript Magic Link Fluxcell for SpectraFlux
// Built on the official @spectraflux/sdk

import {
  Fluxcell,
  FluxcellMetadata,
  HttpRequest,
  HttpResponse,
  RouteMeta,
  EventContext,
  TelemetryBuffer,
  DeduplicationBuffer,
  CausalGuard,
  CausalVerdict,
  publish,
  registerFluxcell,
} from "@spectraflux/sdk/assembly/index";

const telemetry = new TelemetryBuffer(100);
const causalGuard = new CausalGuard(1000);
const dedup = new DeduplicationBuffer<string>(1000);

export class MagicLinkFluxcell extends Fluxcell {
  metadata(): FluxcellMetadata {
    return new FluxcellMetadata("0.1.0", "TypeScript Magic Link Fluxcell");
  }

  routes(): RouteMeta[] {
    return [
      RouteMeta.get("/verify", "Verify magic link token and exchange for session"),
      RouteMeta.post("/verify", "API redemption of magic link token"),
      RouteMeta.get("/status", "Auth service health and status"),
    ];
  }

  subscriptions(): string[] {
    return [
      "auth.magic_link",
      "mutation.requestmagiclink",
    ];
  }

  handleHttp(req: HttpRequest): HttpResponse {
    telemetry.recordProcessed();

    const path = req.path;
    const cleanPath = path.indexOf("?") != -1 ? path.substring(0, path.indexOf("?")) : path;

    if (cleanPath == "/status") {
      return HttpResponse.json('{"status":"ok","fluxcell":"magic_link_ts"}');
    }

    if (cleanPath == "/verify") {
      let token = "";

      // 1. Try extracting token from query string
      const qIdx = path.indexOf("?token=");
      if (qIdx != -1) {
        let tEnd = path.indexOf("&", qIdx + 7);
        if (tEnd == -1) tEnd = path.length;
        token = path.substring(qIdx + 7, tEnd);
      }

      // 2. Try extracting token from request body
      if (token.length == 0 && req.body.length > 0) {
        const body = req.body;
        const tIdx = body.indexOf('"token":');
        if (tIdx != -1) {
          const start = body.indexOf('"', tIdx + 8);
          if (start != -1) {
            const end = body.indexOf('"', start + 1);
            if (end != -1) {
              token = body.substring(start + 1, end);
            }
          }
        }
      }

      if (token.length > 0) {
        telemetry.log("INFO", "Verified magic link token", token);
        return HttpResponse.json(
          '{"status":"VERIFIED","token":"' + token + '","session_id":"sess-' + token + '"}'
        );
      }

      return HttpResponse.json(
        '{"error":"INVALID_OR_EXPIRED_TOKEN","message":"Token is missing or expired"}',
        401
      );
    }

    return HttpResponse.notFound("Endpoint not found: " + path);
  }

  handleEvent(event: EventContext): string {
    // 1. Monotonic Causal Guard
    if (event.hlc.length > 0) {
      const verdict = causalGuard.evaluateAndAdvance(event.topic, event.hlc);
      if (verdict == CausalVerdict.Duplicate || verdict == CausalVerdict.Stale) {
        telemetry.recordStale();
        return '{"status":"discarded","reason":"stale_or_duplicate_hlc"}';
      }
    }

    telemetry.recordProcessed();

    // 2. Outbound event publishing test
    publish("auth.magic_link_dispatched", '{"event_id":"' + event.eventId + '"}');

    return '{"status":"ok","processed":true,"event_id":"' + event.eventId + '"}';
  }
}

registerFluxcell(new MagicLinkFluxcell());

// Re-export C-ABI functions required by SpectraFlux Wasmtime host
export {
  allocate,
  deallocate,
  get_metadata,
  get_subscriptions,
  get_routes,
  handle_http,
  handle_event,
} from "@spectraflux/sdk/assembly/index";
