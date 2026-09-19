//! Zero-Dependency Redis/Valkey Client for SpectraFlux WebAssembly Fluxcells
//!
//! Provides a high-velocity, line-rate abstraction over host-managed Redis and Kevy storage.
//! Eliminates network drivers and socket emulation inside WASM guests while exposing native
//! Redis data structures (Strings, Hashes, Sets, Sorted Sets, Counters).

use serde::de::DeserializeOwned;

#[cfg(target_arch = "wasm32")]
mod host_redis {
    #[link(wasm_import_module = "host_redis")]
    unsafe extern "C" {
        pub fn execute(cmd_ptr: u32, cmd_len: u32, args_ptr: u32, args_len: u32) -> u64;
    }
}

/// Executes a raw Redis command with arguments against the host chassis.
pub fn execute_raw(cmd: &str, args: &[String]) -> Result<serde_json::Value, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let args_json = serde_json::to_string(args)
            .map_err(|e| format!("Failed to serialize redis args: {}", e))?;
        let packed = unsafe {
            host_redis::execute(
                cmd.as_ptr() as usize as u32,
                cmd.len() as u32,
                args_json.as_ptr() as usize as u32,
                args_json.len() as u32,
            )
        };
        let s = crate::abi::read_guest_string(packed);
        let val: serde_json::Value = serde_json::from_str(&s)
            .map_err(|e| format!("Failed to parse host redis response: {}", e))?;
        if let Some(err) = val.get("err").and_then(|e| e.as_str()) {
            return Err(err.to_string());
        }
        Ok(val.get("ok").cloned().unwrap_or(serde_json::Value::Null))
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        // Mock fallback for native unit tests
        let _ = (cmd, args);
        Ok(serde_json::Value::Null)
    }
}

/// Helper to execute a command and deserialize the `ok` payload into `T`.
pub fn execute_typed<T: DeserializeOwned>(cmd: &str, args: &[String]) -> Result<T, String> {
    let val = execute_raw(cmd, args)?;
    serde_json::from_value(val).map_err(|e| format!("Failed to decode Redis response: {}", e))
}

// ---------------------------------------------------------------------------
// Standard Redis Commands
// ---------------------------------------------------------------------------

/// Gets the string value of a key.
pub fn get(key: &str) -> Result<Option<String>, String> {
    let val = execute_raw("GET", &[key.to_string()])?;
    if val.is_null() {
        Ok(None)
    } else {
        Ok(val.as_str().map(|s| s.to_string()))
    }
}

/// Sets the string value of a key with optional TTL in seconds.
pub fn set(key: &str, value: &str, ttl_secs: Option<u64>) -> Result<(), String> {
    let mut args = vec![key.to_string(), value.to_string()];
    if let Some(ttl) = ttl_secs {
        args.push("EX".to_string());
        args.push(ttl.to_string());
    }
    execute_raw("SET", &args)?;
    Ok(())
}

/// Deletes a key from storage.
pub fn del(key: &str) -> Result<bool, String> {
    let res = execute_raw("DEL", &[key.to_string()])?;
    Ok(res.as_u64().map(|c| c > 0).unwrap_or(false))
}

/// Increments the integer value of a key by 1.
pub fn incr(key: &str) -> Result<i64, String> {
    execute_typed("INCR", &[key.to_string()])
}

/// Increments the integer value of a key by given delta.
pub fn incrby(key: &str, delta: i64) -> Result<i64, String> {
    execute_typed("INCRBY", &[key.to_string(), delta.to_string()])
}

/// Gets the value of a hash field.
pub fn hget(key: &str, field: &str) -> Result<Option<String>, String> {
    let val = execute_raw("HGET", &[key.to_string(), field.to_string()])?;
    if val.is_null() {
        Ok(None)
    } else {
        Ok(val.as_str().map(|s| s.to_string()))
    }
}

/// Sets the value of a hash field.
pub fn hset(key: &str, field: &str, value: &str) -> Result<(), String> {
    execute_raw("HSET", &[key.to_string(), field.to_string(), value.to_string()])?;
    Ok(())
}

/// Adds a member to a set.
pub fn sadd(key: &str, member: &str) -> Result<(), String> {
    execute_raw("SADD", &[key.to_string(), member.to_string()])?;
    Ok(())
}

/// Checks if member is in a set.
pub fn sismember(key: &str, member: &str) -> Result<bool, String> {
    let res = execute_raw("SISMEMBER", &[key.to_string(), member.to_string()])?;
    Ok(res.as_u64().map(|c| c > 0).unwrap_or(false))
}

/// Adds a member with score to a sorted set.
pub fn zadd(key: &str, score: f64, member: &str) -> Result<(), String> {
    execute_raw("ZADD", &[key.to_string(), score.to_string(), member.to_string()])?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Fluent Command Builder
// ---------------------------------------------------------------------------

/// Fluent builder for executing custom or advanced Redis commands.
pub struct RedisCommandBuilder {
    command: String,
    args: Vec<String>,
}

impl RedisCommandBuilder {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
        }
    }

    pub fn arg(mut self, arg: impl ToString) -> Self {
        self.args.push(arg.to_string());
        self
    }

    pub fn execute(self) -> Result<serde_json::Value, String> {
        execute_raw(&self.command, &self.args)
    }

    pub fn execute_typed<T: DeserializeOwned>(self) -> Result<T, String> {
        execute_typed(&self.command, &self.args)
    }
}

/// Starts building an arbitrary Redis command.
pub fn cmd(command: impl Into<String>) -> RedisCommandBuilder {
    RedisCommandBuilder::new(command)
}
