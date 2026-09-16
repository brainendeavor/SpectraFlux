// AssemblyScript SDK: Hybrid Logical Clock (HLC) & Monotonic Causality Utilities

export class HlcTuple {
  wall: u64;
  counter: u32;

  constructor(wall: u64, counter: u32) {
    this.wall = wall;
    this.counter = counter;
  }
}

/**
 * Parses an HLC string in the format `<wall_ms>.<logical_counter>` (or `<wall_ms>`).
 */
export function parse_hlc(hlcStr: string): HlcTuple | null {
  const clean = hlcStr.trim();
  if (clean.length == 0) {
    return null;
  }

  const dotIdx = clean.indexOf(".");
  if (dotIdx != -1) {
    const wallStr = clean.substring(0, dotIdx);
    const countStr = clean.substring(dotIdx + 1);
    const wall = U64.parseInt(wallStr);
    const counter = U32.parseInt(countStr);
    return new HlcTuple(wall, counter);
  } else {
    const wall = U64.parseInt(clean);
    return new HlcTuple(wall, 0);
  }
}

/**
 * Returns `true` if the `incoming` HLC is older than or equal to `existing` (i.e. stale or duplicate).
 */
export function is_stale(incoming: string, existing: string): bool {
  const inc = parse_hlc(incoming);
  const ext = parse_hlc(existing);

  if (inc != null && ext != null) {
    if (inc.wall < ext.wall) {
      return true;
    } else if (inc.wall > ext.wall) {
      return false;
    } else {
      return inc.counter <= ext.counter;
    }
  }

  return incoming <= existing;
}
