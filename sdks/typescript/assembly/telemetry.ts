// AssemblyScript SDK: In-Guest Telemetry & Log Ring Buffer

export class TelemetryLogEntry {
  timestamp: string;
  level: string;
  message: string;
  hlc: string | null;

  constructor(level: string, message: string, hlc: string | null = null, timestamp: string = "") {
    this.level = level;
    this.message = message;
    this.hlc = hlc;
    this.timestamp = timestamp;
  }

  toJson(): string {
    let out = '{"level":"' + this.level + '","message":"' + this.message + '"';
    if (this.hlc != null) {
      out += ',"hlc":"' + this.hlc! + '"';
    }
    out += "}";
    return out;
  }
}

export class TelemetryBuffer {
  private processedCount: u64;
  private errorCount: u64;
  private staleCount: u64;
  private capacity: i32;
  private logs: Array<TelemetryLogEntry>;

  constructor(capacity: i32 = 100) {
    this.processedCount = 0;
    this.errorCount = 0;
    this.staleCount = 0;
    this.capacity = capacity;
    this.logs = new Array<TelemetryLogEntry>();
  }

  recordProcessed(): void {
    this.processedCount += 1;
  }

  recordError(): void {
    this.errorCount += 1;
  }

  recordStale(): void {
    this.staleCount += 1;
  }

  log(level: string, message: string, hlc: string | null = null): void {
    if (this.logs.length >= this.capacity) {
      this.logs.shift();
    }
    this.logs.push(new TelemetryLogEntry(level, message, hlc));
  }

  processed(): u64 {
    return this.processedCount;
  }

  errors(): u64 {
    return this.errorCount;
  }

  stale(): u64 {
    return this.staleCount;
  }

  getLogs(): Array<TelemetryLogEntry> {
    return this.logs;
  }

  statsJson(name: string, version: string): string {
    return (
      '{"status":"healthy","fluxcell":"' +
      name +
      '","version":"' +
      version +
      '","total_processed":' +
      this.processedCount.toString() +
      ',"poison_pills_rejected":' +
      this.errorCount.toString() +
      ',"stale_hlc_suppressed":' +
      this.staleCount.toString() +
      "}"
    );
  }
}
