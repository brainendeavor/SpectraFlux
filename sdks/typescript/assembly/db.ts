// AssemblyScript SDK: Transaction-First Host Database Client (`host_db`)

@external("host_db", "begin_tx")
declare function host_begin_tx(db_name_ptr: u32, db_name_len: u32): u64;

@external("host_db", "execute")
declare function host_execute(tx_id: u64, sql_ptr: u32, sql_len: u32, params_ptr: u32, params_len: u32): u64;

@external("host_db", "query")
declare function host_query(tx_id: u64, sql_ptr: u32, sql_len: u32, params_ptr: u32, params_len: u32): u64;

@external("host_db", "commit_tx")
declare function host_commit_tx(tx_id: u64): u64;

@external("host_db", "rollback_tx")
declare function host_rollback_tx(tx_id: u64): u64;

export class FluxTx {
  txId: u64;
  committed: bool;
  rolledBack: bool;

  constructor(txId: u64) {
    this.txId = txId;
    this.committed = false;
    this.rolledBack = false;
  }

  execute(sql: string, paramsJson: string = "[]"): u64 {
    const sqlBuf = String.UTF8.encode(sql);
    const paramsBuf = String.UTF8.encode(paramsJson);

    const packed = host_execute(
      this.txId,
      changetype<u32>(sqlBuf),
      sqlBuf.byteLength as u32,
      changetype<u32>(paramsBuf),
      paramsBuf.byteLength as u32
    );

    const resp = unpackString(packed);
    const errIdx = resp.indexOf('"err":');
    if (errIdx != -1) {
      throw new Error("DB execute failed: " + resp);
    }

    const okIdx = resp.indexOf('"ok":');
    if (okIdx != -1) {
      const numStr = resp.substring(okIdx + 5).replace("}", "").trim();
      return U64.parseInt(numStr);
    }
    return 0;
  }

  query(sql: string, paramsJson: string = "[]"): string {
    const sqlBuf = String.UTF8.encode(sql);
    const paramsBuf = String.UTF8.encode(paramsJson);

    const packed = host_query(
      this.txId,
      changetype<u32>(sqlBuf),
      sqlBuf.byteLength as u32,
      changetype<u32>(paramsBuf),
      paramsBuf.byteLength as u32
    );

    const resp = unpackString(packed);
    const errIdx = resp.indexOf('"err":');
    if (errIdx != -1) {
      throw new Error("DB query failed: " + resp);
    }

    const okIdx = resp.indexOf('"ok":');
    if (okIdx != -1) {
      return resp.substring(okIdx + 5).trim().replace("}", "");
    }
    return "[]";
  }

  commit(): void {
    if (this.committed || this.rolledBack) return;
    const packed = host_commit_tx(this.txId);
    const resp = unpackString(packed);
    const errIdx = resp.indexOf('"err":');
    if (errIdx != -1) {
      throw new Error("DB commit failed: " + resp);
    }
    this.committed = true;
  }

  rollback(): void {
    if (this.committed || this.rolledBack) return;
    const packed = host_rollback_tx(this.txId);
    const resp = unpackString(packed);
    const errIdx = resp.indexOf('"err":');
    if (errIdx != -1) {
      throw new Error("DB rollback failed: " + resp);
    }
    this.rolledBack = true;
  }
}

export class Database {
  name: string;

  constructor(name: string = "") {
    this.name = name;
  }

  static default(): Database {
    return new Database("");
  }

  static named(name: string): Database {
    return new Database(name);
  }

  beginTx(): FluxTx {
    const nameBuf = String.UTF8.encode(this.name);
    const packed = host_begin_tx(
      changetype<u32>(nameBuf),
      nameBuf.byteLength as u32
    );

    const resp = unpackString(packed);
    const errIdx = resp.indexOf('"err":');
    if (errIdx != -1) {
      throw new Error("DB begin_tx failed: " + resp);
    }

    const okIdx = resp.indexOf('"ok":');
    if (okIdx != -1) {
      const numStr = resp.substring(okIdx + 5).replace("}", "").trim();
      const txId = U64.parseInt(numStr);
      return new FluxTx(txId);
    }

    throw new Error("DB begin_tx returned unexpected payload: " + resp);
  }
}

function unpackString(packed: u64): string {
  const ptr = (packed >> 32) as usize;
  const len = (packed & 0xFFFFFFFF) as usize;
  if (len == 0) return "";
  return String.UTF8.decodeUnsafe(ptr, len);
}
