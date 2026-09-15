//! High-Velocity Transactional Host Database Client
//!
//! Provides a safe, ergonomic, RAII-governed transaction interface for Fluxcells.
//! All database operations strictly require an isolated transaction handle (`Transaction`).
//! Uncommitted transactions automatically roll back on `Drop` to prevent leaked locks.

use serde::de::DeserializeOwned;
use serde_json::Value;

#[cfg(target_arch = "wasm32")]
mod host_db {
    #[link(wasm_import_module = "host_db")]
    unsafe extern "C" {
        pub fn begin_tx(db_name_ptr: u32, db_name_len: u32) -> u64;
        pub fn execute(tx_id: u64, sql_ptr: u32, sql_len: u32, params_ptr: u32, params_len: u32) -> u64;
        pub fn query(tx_id: u64, sql_ptr: u32, sql_len: u32, params_ptr: u32, params_len: u32) -> u64;
        pub fn commit_tx(tx_id: u64) -> u64;
        pub fn rollback_tx(tx_id: u64) -> u64;
    }
}

#[allow(dead_code)]
#[derive(serde::Deserialize)]
struct HostResp<T> {
    ok: Option<T>,
    err: Option<String>,
}

#[cfg(target_arch = "wasm32")]
fn parse_host_resp<T: DeserializeOwned>(packed: u64) -> Result<T, String> {
    let s = crate::abi::read_guest_string(packed);
    let resp: HostResp<T> = serde_json::from_str(&s)
        .map_err(|e| format!("Failed to parse host response '{}': {}", s, e))?;
    if let Some(val) = resp.ok {
        Ok(val)
    } else {
        Err(resp.err.unwrap_or_else(|| "Unknown host database error".to_string()))
    }
}

/// Entry point to connect to a named host database pool.
#[derive(Debug, Clone, Default)]
pub struct Database {
    #[allow(dead_code)]
    name: String,
}

impl Database {
    /// Connects to a specific database pool configured in the chassis (e.g. "analytics", "warehouse").
    pub fn open(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// Connects to the default database pool ("default" / primary configured database).
    pub fn default() -> Self {
        Self { name: String::new() }
    }

    /// Acquires a pooled connection from the host chassis and opens an isolated transaction.
    pub fn begin_tx(&self) -> Result<Transaction, String> {
        #[cfg(target_arch = "wasm32")]
        {
            let packed = unsafe {
                host_db::begin_tx(self.name.as_ptr() as u32, self.name.len() as u32)
            };
            let tx_id: u64 = parse_host_resp(packed)?;
            Ok(Transaction {
                id: tx_id,
                committed: false,
                rolled_back: false,
            })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Ok(Transaction {
                id: 1,
                committed: false,
                rolled_back: false,
            })
        }
    }
}

/// An active, isolated database transaction handle.
///
/// Implements RAII safety: if dropped without calling `.commit()`,
/// it will automatically issue an asynchronous `ROLLBACK` on the host
/// to release table and row locks immediately.
pub struct Transaction {
    id: u64,
    committed: bool,
    rolled_back: bool,
}

impl Transaction {
    /// Returns the unique transaction handle ID allocated by the host.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Executes a mutating SQL statement (INSERT, UPDATE, DELETE) inside the transaction.
    ///
    /// Parameters should be provided as a `serde_json::Value` (typically `json!([val1, val2])`).
    pub fn execute(&mut self, sql: &str, params: &Value) -> Result<u64, String> {
        let params_json = params.to_string();
        #[cfg(target_arch = "wasm32")]
        {
            let packed = unsafe {
                host_db::execute(
                    self.id,
                    sql.as_ptr() as u32,
                    sql.len() as u32,
                    params_json.as_ptr() as u32,
                    params_json.len() as u32,
                )
            };
            parse_host_resp(packed)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (sql, params_json);
            Ok(1)
        }
    }

    /// Executes a SELECT query inside the transaction, returning matching rows as dynamic JSON values.
    pub fn query(&mut self, sql: &str, params: &Value) -> Result<Vec<Value>, String> {
        let params_json = params.to_string();
        #[cfg(target_arch = "wasm32")]
        {
            let packed = unsafe {
                host_db::query(
                    self.id,
                    sql.as_ptr() as u32,
                    sql.len() as u32,
                    params_json.as_ptr() as u32,
                    params_json.len() as u32,
                )
            };
            let rows_str: String = parse_host_resp(packed)?;
            serde_json::from_str(&rows_str)
                .map_err(|e| format!("Failed to parse query rows '{}': {}", rows_str, e))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (sql, params_json);
            Ok(Vec::new())
        }
    }

    /// Executes a SELECT query inside the transaction and automatically deserializes rows into `Vec<T>`.
    pub fn query_as<T: DeserializeOwned>(&mut self, sql: &str, params: &Value) -> Result<Vec<T>, String> {
        let rows = self.query(sql, params)?;
        let val = Value::Array(rows);
        serde_json::from_value(val).map_err(|e| format!("Failed to deserialize query result: {}", e))
    }

    /// Explicitly commits the transaction, persisting all changes to the database.
    pub fn commit(mut self) -> Result<(), String> {
        self.committed = true;
        #[cfg(target_arch = "wasm32")]
        {
            let packed = unsafe { host_db::commit_tx(self.id) };
            let _: Option<Value> = parse_host_resp(packed)?;
            Ok(())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Ok(())
        }
    }

    /// Explicitly rolls back the transaction, discarding all uncommitted changes.
    pub fn rollback(mut self) -> Result<(), String> {
        self.rolled_back = true;
        #[cfg(target_arch = "wasm32")]
        {
            let packed = unsafe { host_db::rollback_tx(self.id) };
            let _: Option<Value> = parse_host_resp(packed)?;
            Ok(())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Ok(())
        }
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        if !self.committed && !self.rolled_back {
            #[cfg(target_arch = "wasm32")]
            unsafe {
                let _ = host_db::rollback_tx(self.id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transaction_lifecycle_native() {
        let db = Database::default();
        let mut tx = db.begin_tx().expect("Failed to begin native tx mock");
        let affected = tx.execute("UPDATE test SET x = 1", &serde_json::json!([])).unwrap();
        assert_eq!(affected, 1);
        tx.commit().expect("Commit should succeed on native mock");
    }

    #[test]
    fn test_transaction_auto_rollback_drop() {
        let db = Database::open("analytics");
        let tx = db.begin_tx().unwrap();
        // Drop without commit should safely trigger rollback
        drop(tx);
    }
}
