use anyhow::{anyhow, Result};
use std::sync::Arc;
#[cfg(feature = "kevy")]
use std::time::Duration;

#[async_trait::async_trait]
pub trait FluxStorage: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<String>>;
    async fn set(&self, key: &str, value: &str, ttl_seconds: u64) -> Result<bool>;
    async fn delete(&self, key: &str) -> Result<bool>;
    async fn get_del(&self, key: &str) -> Result<Option<String>>;
    async fn execute_cmd(&self, cmd: &str, args: &[String]) -> Result<serde_json::Value>;
}

#[cfg(feature = "kevy")]
pub struct KevyStorage {
    store: Arc<kevy_embedded::Store>,
}

#[cfg(feature = "kevy")]
impl KevyStorage {
    pub fn new_in_memory() -> Result<Self> {
        let config = kevy_embedded::Config::default();
        let store = kevy_embedded::Store::open(config)
            .map_err(|e| anyhow!("Failed to open kevy-embedded store: {:?}", e))?;
        Ok(Self {
            store: Arc::new(store),
        })
    }

    pub fn new_with_persist(path: &str) -> Result<Self> {
        let path_obj = std::path::Path::new(path);
        let mut target_path = path.to_string();

        if let Some(parent) = path_obj.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    if path.starts_with("/data/") || path == "/data" {
                        log::warn!(
                            "Cannot create root volume directory '{}' ({:?}). Falling back to local './data/' directory.",
                            parent.display(),
                            e
                        );
                        let relative_path = format!(".{}", path);
                        if let Some(rel_parent) = std::path::Path::new(&relative_path).parent() {
                            let _ = std::fs::create_dir_all(rel_parent);
                        }
                        target_path = relative_path;
                    } else {
                        return Err(anyhow!("Failed to create storage directory '{}': {:?}", parent.display(), e));
                    }
                }
            }
        }

        let config = kevy_embedded::Config::default().with_persist(&target_path);
        let store = match kevy_embedded::Store::open(config) {
            Ok(s) => s,
            Err(e) if target_path.starts_with("/data/") => {
                log::warn!(
                    "Failed to open persistent store at '{}' ({:?}). Falling back to local './data/' directory.",
                    target_path,
                    e
                );
                let fallback_path = format!(".{}", target_path);
                if let Some(rel_parent) = std::path::Path::new(&fallback_path).parent() {
                    let _ = std::fs::create_dir_all(rel_parent);
                }
                kevy_embedded::Store::open(kevy_embedded::Config::default().with_persist(&fallback_path))
                    .map_err(|err| anyhow!("Failed to open persistent kevy store at fallback {}: {:?}", fallback_path, err))?
            }
            Err(e) => return Err(anyhow!("Failed to open persistent kevy-embedded store at {}: {:?}", target_path, e)),
        };

        Ok(Self {
            store: Arc::new(store),
        })
    }
}

#[cfg(feature = "kevy")]
#[async_trait::async_trait]
impl FluxStorage for KevyStorage {
    async fn get(&self, key: &str) -> Result<Option<String>> {
        let res = self.store.get(key.as_bytes())
            .map_err(|e| anyhow!("kevy get error: {:?}", e))?;
        Ok(res.and_then(|bytes| String::from_utf8(bytes).ok()))
    }

    async fn set(&self, key: &str, value: &str, ttl_seconds: u64) -> Result<bool> {
        if ttl_seconds > 0 {
            let ttl = Duration::from_secs(ttl_seconds);
            self.store.set_with_ttl(key.as_bytes(), value.as_bytes(), ttl)
                .map_err(|e| anyhow!("kevy set_with_ttl error: {:?}", e))
        } else {
            self.store.set(key.as_bytes(), value.as_bytes())
                .map_err(|e| anyhow!("kevy set error: {:?}", e))
        }
    }

    async fn delete(&self, key: &str) -> Result<bool> {
        let count = self.store.del(&[key.as_bytes()])
            .map_err(|e| anyhow!("kevy del error: {:?}", e))?;
        Ok(count > 0)
    }

    async fn get_del(&self, key: &str) -> Result<Option<String>> {
        let res = self.store.getdel(key.as_bytes())
            .map_err(|e| anyhow!("kevy getdel error: {:?}", e))?;
        Ok(res.and_then(|bytes| String::from_utf8(bytes).ok()))
    }

    async fn execute_cmd(&self, cmd: &str, args: &[String]) -> Result<serde_json::Value> {
        let upper = cmd.to_ascii_uppercase();
        match upper.as_str() {
            "GET" => {
                if args.is_empty() {
                    return Err(anyhow!("GET requires 1 argument"));
                }
                let res = self.get(&args[0]).await?;
                Ok(res.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null))
            }
            "SET" => {
                if args.len() < 2 {
                    return Err(anyhow!("SET requires at least 2 arguments"));
                }
                let mut ttl = 0;
                if args.len() >= 4 && args[2].eq_ignore_ascii_case("EX") {
                    ttl = args[3].parse().unwrap_or(0);
                }
                let ok = self.set(&args[0], &args[1], ttl).await?;
                Ok(if ok { serde_json::Value::String("OK".to_string()) } else { serde_json::Value::Null })
            }
            "DEL" => {
                if args.is_empty() {
                    return Err(anyhow!("DEL requires at least 1 argument"));
                }
                let mut count = 0;
                for arg in args {
                    if self.delete(arg).await? {
                        count += 1;
                    }
                }
                Ok(serde_json::json!(count))
            }
            "GETDEL" => {
                if args.is_empty() {
                    return Err(anyhow!("GETDEL requires 1 argument"));
                }
                let res = self.get_del(&args[0]).await?;
                Ok(res.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null))
            }
            "INCR" => {
                if args.is_empty() {
                    return Err(anyhow!("INCR requires 1 argument"));
                }
                let key = &args[0];
                let current = self.get(key).await?.and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
                let next = current + 1;
                self.set(key, &next.to_string(), 0).await?;
                Ok(serde_json::json!(next))
            }
            "INCRBY" => {
                if args.len() < 2 {
                    return Err(anyhow!("INCRBY requires 2 arguments"));
                }
                let key = &args[0];
                let delta: i64 = args[1].parse().map_err(|e| anyhow!("Invalid integer for INCRBY: {}", e))?;
                let current = self.get(key).await?.and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
                let next = current + delta;
                self.set(key, &next.to_string(), 0).await?;
                Ok(serde_json::json!(next))
            }
            _ => Err(anyhow!("Unsupported in-process Kevy command: '{}'", cmd)),
        }
    }
}

pub struct RedisStorage {
    connection_manager: redis::aio::ConnectionManager,
}

impl RedisStorage {
    pub async fn new(redis_url: &str) -> Result<Self> {
        let client = redis::Client::open(redis_url)?;
        let connection_manager = redis::aio::ConnectionManager::new(client).await?;
        Ok(Self { connection_manager })
    }
}

#[async_trait::async_trait]
impl FluxStorage for RedisStorage {
    async fn get(&self, key: &str) -> Result<Option<String>> {
        let mut conn = self.connection_manager.clone();
        let res: Option<String> = redis::cmd("GET")
            .arg(key)
            .query_async(&mut conn)
            .await?;
        Ok(res)
    }

    async fn set(&self, key: &str, value: &str, ttl_seconds: u64) -> Result<bool> {
        let mut conn = self.connection_manager.clone();
        let mut cmd = redis::cmd("SET");
        cmd.arg(key).arg(value);
        if ttl_seconds > 0 {
            cmd.arg("EX").arg(ttl_seconds);
        }
        let res: Option<String> = cmd.query_async(&mut conn).await?;
        Ok(res.is_some())
    }

    async fn delete(&self, key: &str) -> Result<bool> {
        let mut conn = self.connection_manager.clone();
        let deleted: u64 = redis::cmd("DEL")
            .arg(key)
            .query_async(&mut conn)
            .await?;
        Ok(deleted > 0)
    }

    async fn get_del(&self, key: &str) -> Result<Option<String>> {
        let mut conn = self.connection_manager.clone();
        let res: Option<String> = redis::cmd("GETDEL")
            .arg(key)
            .query_async(&mut conn)
            .await?;
        Ok(res)
    }

    async fn execute_cmd(&self, cmd: &str, args: &[String]) -> Result<serde_json::Value> {
        let mut conn = self.connection_manager.clone();
        let mut c = redis::cmd(cmd);
        for arg in args {
            c.arg(arg);
        }
        let res: redis::Value = c.query_async(&mut conn).await?;
        Ok(redis_value_to_json(res))
    }
}

fn redis_value_to_json(val: redis::Value) -> serde_json::Value {
    match val {
        redis::Value::Nil => serde_json::Value::Null,
        redis::Value::Int(i) => serde_json::json!(i),
        redis::Value::Data(bytes) => {
            if let Ok(s) = String::from_utf8(bytes.clone()) {
                serde_json::Value::String(s)
            } else {
                serde_json::json!(bytes)
            }
        }
        redis::Value::Bulk(items) => {
            let json_items: Vec<serde_json::Value> = items.into_iter().map(redis_value_to_json).collect();
            serde_json::Value::Array(json_items)
        }
        redis::Value::Status(s) => serde_json::Value::String(s),
        redis::Value::Okay => serde_json::Value::String("OK".to_string()),
    }
}

/// Create a FluxStorage instance from a standardized URL scheme:
/// - `kevy://embedded` or `kevy://memory`: In-memory Kevy key-value store
/// - `kevy:///path/to/persist.kevy`: Persistent file-backed Kevy store
/// - `redis://...` or `rediss://...`: External Redis / Valkey cluster
/// - `valkey://...` or `valkeys://...`: Normalized to redis:// and connects via Redis protocol
pub async fn create_storage_from_url(url: &str) -> Result<Arc<dyn FluxStorage>> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Empty storage URL provided"));
    }

    if trimmed == "kevy://embedded" || trimmed == "kevy://memory" || trimmed == "embedded_kevy" || trimmed == "kevy" {
        #[cfg(feature = "kevy")]
        {
            return Ok(Arc::new(KevyStorage::new_in_memory()?));
        }
        #[cfg(not(feature = "kevy"))]
        {
            return Err(anyhow!("kevy feature is not enabled"));
        }
    }

    if let Some(path) = trimmed.strip_prefix("kevy://") {
        #[cfg(feature = "kevy")]
        {
            if path == "embedded" || path == "memory" {
                return Ok(Arc::new(KevyStorage::new_in_memory()?));
            }
            return Ok(Arc::new(KevyStorage::new_with_persist(path)?));
        }
        #[cfg(not(feature = "kevy"))]
        {
            return Err(anyhow!("kevy feature is not enabled"));
        }
    }

    if trimmed.starts_with("redis://") || trimmed.starts_with("rediss://") {
        return Ok(Arc::new(RedisStorage::new(trimmed).await?));
    }

    if let Some(rest) = trimmed.strip_prefix("valkey://") {
        let redis_url = format!("redis://{}", rest);
        return Ok(Arc::new(RedisStorage::new(&redis_url).await?));
    }

    if let Some(rest) = trimmed.strip_prefix("valkeys://") {
        let rediss_url = format!("rediss://{}", rest);
        return Ok(Arc::new(RedisStorage::new(&rediss_url).await?));
    }

    Err(anyhow!(
        "Unsupported storage URL scheme: '{}'. Expected kevy://, redis://, rediss://, or valkey://",
        trimmed
    ))
}

pub async fn create_storage(backend: &str, addr: Option<&str>) -> Result<Arc<dyn FluxStorage>> {
    match backend {
        #[cfg(feature = "kevy")]
        "embedded_kevy" | "kevy" => {
            if let Some(path) = addr.filter(|p| !p.starts_with("redis://") && !p.starts_with("valkey://") && !p.is_empty()) {
                if path.starts_with("kevy://") {
                    create_storage_from_url(path).await
                } else {
                    Ok(Arc::new(KevyStorage::new_with_persist(path)?))
                }
            } else {
                Ok(Arc::new(KevyStorage::new_in_memory()?))
            }
        }
        "redis" | "valkey" => {
            let url = addr.unwrap_or("redis://127.0.0.1:6379");
            create_storage_from_url(url).await
        }
        url if url.starts_with("kevy://") || url.starts_with("redis://") || url.starts_with("rediss://") || url.starts_with("valkey://") || url.starts_with("valkeys://") => {
            create_storage_from_url(url).await
        }
        other => Err(anyhow!("Unsupported storage backend: '{}'", other)),
    }
}

#[cfg(all(test, feature = "kevy"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_kevy_storage_basic_and_getdel() {
        let storage = KevyStorage::new_in_memory().unwrap();
        
        // Basic set & get
        let ok = storage.set("magic_link:tok123", "user@example.com", 60).await.unwrap();
        assert!(ok);
        let val = storage.get("magic_link:tok123").await.unwrap();
        assert_eq!(val.as_deref(), Some("user@example.com"));

        // Atomic getdel
        let redeemed = storage.get_del("magic_link:tok123").await.unwrap();
        assert_eq!(redeemed.as_deref(), Some("user@example.com"));

        // Second getdel should return None (already redeemed/deleted)
        let redeemed_again = storage.get_del("magic_link:tok123").await.unwrap();
        assert_eq!(redeemed_again, None);
    }

    #[tokio::test]
    async fn test_kevy_storage_non_existent_key_operations() {
        let storage = KevyStorage::new_in_memory().unwrap();

        // Getting a missing key returns None
        let val = storage.get("missing_key_xyz").await.unwrap();
        assert_eq!(val, None);

        // Deleting a missing key returns false
        let deleted = storage.delete("missing_key_xyz").await.unwrap();
        assert!(!deleted);

        // getdel on missing key returns None
        let getdel_val = storage.get_del("missing_key_xyz").await.unwrap();
        assert_eq!(getdel_val, None);
    }

    #[tokio::test]
    async fn test_kevy_storage_overwrite_and_multi_key_isolation() {
        let storage = KevyStorage::new_in_memory().unwrap();

        // Set initial key
        assert!(storage.set("cell:key_a", "val_1", 0).await.unwrap());
        assert_eq!(storage.get("cell:key_a").await.unwrap().as_deref(), Some("val_1"));

        // Overwrite key
        assert!(storage.set("cell:key_a", "val_2", 0).await.unwrap());
        assert_eq!(storage.get("cell:key_a").await.unwrap().as_deref(), Some("val_2"));

        // Set distinct key B
        assert!(storage.set("cell:key_b", "val_b", 0).await.unwrap());
        assert_eq!(storage.get("cell:key_a").await.unwrap().as_deref(), Some("val_2"));
        assert_eq!(storage.get("cell:key_b").await.unwrap().as_deref(), Some("val_b"));

        // Delete key A, verify key B unaffected
        assert!(storage.delete("cell:key_a").await.unwrap());
        assert_eq!(storage.get("cell:key_a").await.unwrap(), None);
        assert_eq!(storage.get("cell:key_b").await.unwrap().as_deref(), Some("val_b"));
    }

    #[tokio::test]
    async fn test_kevy_storage_ttl_expiration() {
        let storage = KevyStorage::new_in_memory().unwrap();

        // Set key with 1 second TTL
        storage.set("ephemeral_token", "temporary_secret", 1).await.unwrap();
        assert_eq!(storage.get("ephemeral_token").await.unwrap().as_deref(), Some("temporary_secret"));

        // Wait 1.1s for TTL expiry
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

        // Key should now be expired and return None
        let expired_val = storage.get("ephemeral_token").await.unwrap();
        assert_eq!(expired_val, None);
    }

    #[tokio::test]
    async fn test_kevy_storage_persistence() {
        let temp_dir = std::env::temp_dir().join(format!("spectra_kevy_test_{}", uuid::Uuid::new_v4()));
        let db_path = temp_dir.to_str().unwrap().to_string();

        {
            let storage = KevyStorage::new_with_persist(&db_path).unwrap();
            storage.set("persisted_token", "persisted_value", 0).await.unwrap();
            let val = storage.get("persisted_token").await.unwrap();
            assert_eq!(val.as_deref(), Some("persisted_value"));
        }

        // Re-open from same path
        {
            let storage = KevyStorage::new_with_persist(&db_path).unwrap();
            let val = storage.get("persisted_token").await.unwrap();
            assert_eq!(val.as_deref(), Some("persisted_value"));
        }

        // Clean up
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_create_storage_factory() {
        let storage_kevy = create_storage("embedded_kevy", None).await;
        assert!(storage_kevy.is_ok());

        let storage_alias = create_storage("kevy", None).await;
        assert!(storage_alias.is_ok());

        let storage_invalid = create_storage("unknown_driver_xyz", None).await;
        match storage_invalid {
            Err(e) => assert!(e.to_string().contains("Unsupported storage backend")),
            Ok(_) => panic!("Expected storage_invalid to fail"),
        }
    }

    #[tokio::test]
    async fn test_storage_adversarial_corrupted_disk_file() {
        let temp_dir = std::env::temp_dir().join(format!("spectra_corrupt_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let db_file = temp_dir.join("corrupted.db");
        // Write raw garbage into the persistent file
        std::fs::write(&db_file, b"TOTAL_CORRUPTED_GARBAGE_RANDOM_BYTES_HEADER_12345").unwrap();

        let db_path = db_file.to_str().unwrap();
        // Opening should either error or safely handle without panicking or segfaulting
        let res = KevyStorage::new_with_persist(db_path);
        // Ensure result is handled safely
        match res {
            Ok(store) => {
                // If it opened by re-initializing fresh state, ensure it doesn't crash on operations
                let _ = store.get("test").await;
            }
            Err(_) => {
                // Properly detected file corruption and rejected
            }
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_storage_adversarial_high_concurrency_race() {
        let storage = Arc::new(KevyStorage::new_in_memory().unwrap());
        let num_tasks = 40;
        let iterations = 50;

        let mut handles = Vec::new();

        for task_id in 0..num_tasks {
            let store_clone = storage.clone();
            handles.push(tokio::spawn(async move {
                for i in 0..iterations {
                    let key = format!("race:key:{}", (task_id + i) % 5);
                    let val = format!("val-{}-{}", task_id, i);

                    let _ = store_clone.set(&key, &val, 0).await;
                    let _ = store_clone.get(&key).await;

                    if i % 3 == 0 {
                        let _ = store_clone.get_del(&key).await;
                    }
                    if i % 5 == 0 {
                        let _ = store_clone.delete(&key).await;
                    }
                }
            }));
        }

        for h in handles {
            h.await.unwrap();
        }
    }

    #[tokio::test]
    async fn test_storage_adversarial_large_payload() {
        let storage = KevyStorage::new_in_memory().unwrap();
        // 2MB payload
        let large_str = "A".repeat(2 * 1024 * 1024);

        assert!(storage.set("large_key", &large_str, 0).await.unwrap());
        let retrieved = storage.get("large_key").await.unwrap();
        assert_eq!(retrieved.map(|s| s.len()), Some(2 * 1024 * 1024));
    }

    #[tokio::test]
    async fn test_storage_adversarial_special_key_characters() {
        let storage = KevyStorage::new_in_memory().unwrap();
        let weird_keys = vec![
            "user:email+tag@example.com/oauth/callback",
            "emoji:🚀:auth:🔥",
            "spaces and tabs \t \r in key",
            "unicode:日本語:ключ:مفتاح",
        ];

        for k in &weird_keys {
            assert!(storage.set(k, "valid_value", 0).await.unwrap());
            let val = storage.get(k).await.unwrap();
            assert_eq!(val.as_deref(), Some("valid_value"));
            assert!(storage.delete(k).await.unwrap());
            assert_eq!(storage.get(k).await.unwrap(), None);
        }
    }

    #[tokio::test]
    async fn test_kevy_storage_execute_cmd() {
        let storage = KevyStorage::new_in_memory().unwrap();

        // 1. SET & GET
        let set_res = storage.execute_cmd("SET", &["test_counter".to_string(), "10".to_string()]).await.unwrap();
        assert_eq!(set_res, "OK");

        let get_res = storage.execute_cmd("GET", &["test_counter".to_string()]).await.unwrap();
        assert_eq!(get_res, "10");

        // 2. INCR
        let incr_res = storage.execute_cmd("INCR", &["test_counter".to_string()]).await.unwrap();
        assert_eq!(incr_res, 11);

        // 3. INCRBY
        let incrby_res = storage.execute_cmd("INCRBY", &["test_counter".to_string(), "5".to_string()]).await.unwrap();
        assert_eq!(incrby_res, 16);

        // 4. DEL
        let del_res = storage.execute_cmd("DEL", &["test_counter".to_string()]).await.unwrap();
        assert_eq!(del_res, 1);

        let get_after_del = storage.execute_cmd("GET", &["test_counter".to_string()]).await.unwrap();
        assert!(get_after_del.is_null());
    }

    #[tokio::test]
    async fn test_create_storage_from_url_schemes() {
        // 1. kevy://embedded
        let s1 = create_storage_from_url("kevy://embedded").await;
        assert!(s1.is_ok());
        let store1 = s1.unwrap();
        assert!(store1.set("foo", "bar", 0).await.unwrap());
        assert_eq!(store1.get("foo").await.unwrap().as_deref(), Some("bar"));

        // 2. kevy://memory
        let s2 = create_storage_from_url("kevy://memory").await;
        assert!(s2.is_ok());
        let store2 = s2.unwrap();
        assert_eq!(store2.get("foo").await.unwrap(), None); // Isolated store

        // 3. kevy:///tmp/path...
        let temp_dir = std::env::temp_dir().join(format!("spectra_url_test_{}", uuid::Uuid::new_v4()));
        let file_path = temp_dir.join("test.kevy");
        let url = format!("kevy://{}", file_path.to_str().unwrap());
        let s3 = create_storage_from_url(&url).await;
        assert!(s3.is_ok());
        let store3 = s3.unwrap();
        assert!(store3.set("persisted_url_key", "persisted_url_val", 0).await.unwrap());
        assert_eq!(store3.get("persisted_url_key").await.unwrap().as_deref(), Some("persisted_url_val"));
        let _ = std::fs::remove_dir_all(&temp_dir);

        // 4. Invalid scheme
        let err = create_storage_from_url("sqlite:///tmp/db.sqlite").await;
        match err {
            Err(e) => assert!(e.to_string().contains("Unsupported storage URL scheme")),
            Ok(_) => panic!("Expected error for invalid URL scheme"),
        }

        // 5. Empty URL
        let empty_err = create_storage_from_url("   ").await;
        match empty_err {
            Err(e) => assert!(e.to_string().contains("Empty storage URL provided")),
            Ok(_) => panic!("Expected error for empty storage URL"),
        }
    }

    #[tokio::test]
    async fn test_create_storage_from_url_data_volume_fallback() {
        // Testing kevy:///data/fluxcell-storage.kevy in non-root environment
        let s = create_storage_from_url("kevy:///data/fluxcell-storage-test.kevy").await;
        assert!(s.is_ok(), "Expected /data/ to either succeed or fall back gracefully to ./data/");
        let store = s.unwrap();
        assert!(store.set("test_data_key", "test_data_val", 0).await.unwrap());
        assert_eq!(store.get("test_data_key").await.unwrap().as_deref(), Some("test_data_val"));
        // Clean up local fallback if created
        if std::path::Path::new("./data/fluxcell-storage-test.kevy").is_dir() {
            let _ = std::fs::remove_dir_all("./data/fluxcell-storage-test.kevy");
        } else if std::path::Path::new("./data/fluxcell-storage-test.kevy").exists() {
            let _ = std::fs::remove_file("./data/fluxcell-storage-test.kevy");
        }
        if std::path::Path::new("./data").is_dir() && std::fs::read_dir("./data").map(|mut it| it.next().is_none()).unwrap_or(false) {
            let _ = std::fs::remove_dir("./data");
        }
    }
}

