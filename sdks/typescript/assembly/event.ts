// AssemblyScript SDK: Event Domain Models
import { getStep, saveStep, step } from "./checkpoint";
import { unescapeJson } from "./http";

export enum EventVerdict {
  Ack = 0,
  Nack = 1,
  DeadLetter = 2,
}

export class EventContext {
  eventId: string;
  hlc: string;
  topic: string;
  payloadJson: string;

  constructor(eventId: string, hlc: string, topic: string, payloadJson: string) {
    this.eventId = eventId;
    this.hlc = hlc;
    this.topic = topic;
    this.payloadJson = payloadJson;
  }

  static fromJson(jsonStr: string): EventContext {
    let eventId = "";
    let hlc = "";
    let topic = "";
    let payload = "";

    // Extract event_id / event-id
    let idIdx = jsonStr.indexOf('"event_id":');
    if (idIdx == -1) idIdx = jsonStr.indexOf('"event-id":');
    if (idIdx != -1) {
      const start = jsonStr.indexOf('"', idIdx + 11);
      if (start != -1) {
        const end = jsonStr.indexOf('"', start + 1);
        if (end != -1) eventId = jsonStr.substring(start + 1, end);
      }
    }

    // Extract hlc
    const hlcIdx = jsonStr.indexOf('"hlc":');
    if (hlcIdx != -1) {
      const start = jsonStr.indexOf('"', hlcIdx + 6);
      if (start != -1) {
        const end = jsonStr.indexOf('"', start + 1);
        if (end != -1) hlc = jsonStr.substring(start + 1, end);
      }
    }

    // Extract topic
    const topicIdx = jsonStr.indexOf('"topic":');
    if (topicIdx != -1) {
      const start = jsonStr.indexOf('"', topicIdx + 8);
      if (start != -1) {
        const end = jsonStr.indexOf('"', start + 1);
        if (end != -1) topic = jsonStr.substring(start + 1, end);
      }
    }

    // Extract payload / payload_json
    let payloadIdx = jsonStr.indexOf('"payload_json":');
    let keyLen = 15;
    if (payloadIdx == -1) {
      payloadIdx = jsonStr.indexOf('"payload":');
      keyLen = 10;
    }
    if (payloadIdx != -1) {
      const rest = jsonStr.substring(payloadIdx + keyLen).trimStart();
      if (rest.startsWith('"')) {
        let inEscape = false;
        let end = -1;
        for (let i = 1; i < rest.length; i++) {
          const c = rest.charCodeAt(i);
          if (inEscape) {
            inEscape = false;
          } else if (c == 92) {
            inEscape = true;
          } else if (c == 34) {
            end = i;
            break;
          }
        }
        if (end != -1) {
          payload = unescapeJson(rest.substring(1, end));
        } else {
          payload = rest;
        }
      } else {
        payload = rest;
        if (payload.endsWith('}')) {
          payload = payload.substring(0, payload.length - 1).trim();
        }
      }
    } else {
      payload = jsonStr;
    }

    return new EventContext(eventId, hlc, topic, payload);
  }

  /**
   * Executes a durable, idempotent step within the context of this event.
   * If the step has already executed for this command/event ID,
   * returns the cached result immediately without invoking `action`.
   */
  step(stepName: string, action: () => string, ttlSeconds: u64 = 86400): string {
    return step(stepName, action, ttlSeconds);
  }

  /**
   * Retrieves the cached result of a previously executed step for this event.
   */
  getStep(stepName: string): string | null {
    return getStep(stepName);
  }

  /**
   * Manually memoizes a step result for this event.
   */
  saveStep(stepName: string, resultJson: string, ttlSeconds: u64 = 86400): bool {
    return saveStep(stepName, resultJson, ttlSeconds);
  }
}

