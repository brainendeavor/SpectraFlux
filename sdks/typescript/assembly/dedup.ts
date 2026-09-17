// AssemblyScript SDK: Fast In-Memory Monotonic HLC Deduplication Buffer

import { is_stale } from "./hlc";

export class DeduplicationEntry<V> {
  hlc: string;
  val: V;

  constructor(hlc: string, val: V) {
    this.hlc = hlc;
    this.val = val;
  }
}

/**
 * Bounded deduplication buffer mapping string keys to their latest observed HLC timestamp and value.
 */
export class DeduplicationBuffer<V> {
  private capacity: i32;
  private entries: Map<string, DeduplicationEntry<V>>;
  private keys: Array<string>;

  constructor(capacity: i32 = 1000) {
    this.capacity = capacity;
    this.entries = new Map<string, DeduplicationEntry<V>>();
    this.keys = new Array<string>();
  }

  /**
   * Evaluates if an incoming HLC timestamp is strictly newer than the cached HLC for `key`.
   *
   * If fresh: updates the cache with `(hlc, val)` and returns `true`.
   * If stale or duplicate: leaves the cache untouched and returns `false`.
   */
  checkAndUpdate(key: string, hlc: string, val: V): bool {
    if (this.entries.has(key)) {
      const existing = this.entries.get(key);
      if (is_stale(hlc, existing.hlc)) {
        return false;
      }
    } else {
      if (this.entries.size >= this.capacity) {
        if (this.keys.length > 0) {
          const oldest = this.keys.shift();
          this.entries.delete(oldest);
        }
      }
      this.keys.push(key);
    }

    this.entries.set(key, new DeduplicationEntry<V>(hlc, val));
    return true;
  }

  /**
   * Returns the cached entry for `key` if present.
   */
  get(key: string): DeduplicationEntry<V> | null {
    if (this.entries.has(key)) {
      return this.entries.get(key);
    }
    return null;
  }

  clear(): void {
    this.entries.clear();
    this.keys = new Array<string>();
  }

  size(): i32 {
    return this.entries.size;
  }
}
