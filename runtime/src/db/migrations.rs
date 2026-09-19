use anyhow::{Context, Result};
use std::collections::HashSet;
use std::sync::Arc;

/// A statically embedded database migration compatible with dbmate.
#[derive(Debug, Clone)]
pub struct Migration {
    pub version: &'static str,
    pub name: &'static str,
    pub raw_sql: &'static str,
}

impl Migration {
    pub const fn new(version: &'static str, name: &'static str, raw_sql: &'static str) -> Self {
        Self {
            version,
            name,
            raw_sql,
        }
    }

    /// Extracts the SQL statements designated for the `up` migration.
    pub fn up_sql(&self) -> &str {
        extract_directive_sql(self.raw_sql, "-- migrate:up", "-- migrate:down")
    }

    /// Extracts the SQL statements designated for the `down` migration.
    pub fn down_sql(&self) -> &str {
        extract_directive_sql(self.raw_sql, "-- migrate:down", "-- migrate:up")
    }
}

fn extract_directive_sql<'a>(raw: &'a str, target: &str, opposite: &str) -> &'a str {
    if let Some(target_idx) = raw.find(target) {
        let content_start = target_idx + target.len();
        let rest = &raw[content_start..];
        if let Some(opposite_idx) = rest.find(opposite) {
            rest[..opposite_idx].trim()
        } else {
            rest.trim()
        }
    } else {
        raw.trim()
    }
}

/// The registry of embedded core migrations for SpectraFlux.
pub const EMBEDDED_MIGRATIONS: &[Migration] = &[
    Migration::new(
        "20260919000001",
        "create_auth_user_roles",
        include_str!("../../../db/migrations/20260919000001_create_auth_user_roles.sql"),
    ),
];

/// Idempotently applies all pending embedded migrations against PostgreSQL using the dbmate protocol.
#[cfg(feature = "postgres")]
pub async fn run_embedded_migrations(pool: &Arc<deadpool_postgres::Pool>) -> Result<usize> {
    let mut client = pool
        .get()
        .await
        .context("Failed to acquire PostgreSQL connection from pool for migrations")?;

    // 1. Ensure dbmate standard schema_migrations table exists
    client
        .batch_execute(
            r#"
            CREATE TABLE IF NOT EXISTS schema_migrations (
                version VARCHAR(255) PRIMARY KEY
            );
            "#,
        )
        .await
        .context("Failed to ensure schema_migrations table exists")?;

    // 2. Query already applied migrations
    let rows = client
        .query("SELECT version FROM schema_migrations", &[])
        .await
        .context("Failed to query schema_migrations table")?;

    let applied: HashSet<String> = rows.into_iter().map(|r| r.get::<_, String>(0)).collect();

    // 3. Sort migrations chronologically
    let mut migrations = EMBEDDED_MIGRATIONS.to_vec();
    migrations.sort_by_key(|m| m.version);

    let mut applied_count = 0;

    for m in migrations {
        if applied.contains(m.version) {
            continue;
        }

        let up_sql = m.up_sql();
        if up_sql.is_empty() {
            continue;
        }

        log::info!(
            "Running database migration: {} ({}) ...",
            m.version,
            m.name
        );

        let tx = client
            .transaction()
            .await
            .with_context(|| format!("Failed to start transaction for migration {}", m.version))?;

        tx.batch_execute(up_sql)
            .await
            .with_context(|| format!("Failed to execute DDL for migration {}", m.version))?;

        tx.execute(
            "INSERT INTO schema_migrations (version) VALUES ($1)",
            &[&m.version],
        )
        .await
        .with_context(|| format!("Failed to record version {} in schema_migrations", m.version))?;

        tx.commit()
            .await
            .with_context(|| format!("Failed to commit transaction for migration {}", m.version))?;

        log::info!(
            "Successfully applied database migration: {} ({})",
            m.version,
            m.name
        );
        applied_count += 1;
    }

    if applied_count == 0 {
        log::info!("Database schema is up-to-date (no pending migrations)");
    } else {
        log::info!("Applied {} pending database migration(s)", applied_count);
    }

    Ok(applied_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_directive_sql() {
        let raw = r#"
        -- migrate:up
        CREATE TABLE test_table (id INT);
        CREATE INDEX idx_test ON test_table(id);

        -- migrate:down
        DROP TABLE test_table;
        "#;

        let migration = Migration::new("20260101000001", "test_mig", raw);
        let up = migration.up_sql();
        assert!(up.contains("CREATE TABLE test_table"));
        assert!(up.contains("CREATE INDEX idx_test"));
        assert!(!up.contains("DROP TABLE"));

        let down = migration.down_sql();
        assert!(down.contains("DROP TABLE test_table"));
        assert!(!down.contains("CREATE TABLE test_table"));
    }

    #[test]
    fn test_embedded_migrations_validity() {
        assert!(!EMBEDDED_MIGRATIONS.is_empty());
        for m in EMBEDDED_MIGRATIONS {
            assert!(!m.version.is_empty());
            assert!(!m.name.is_empty());
            let up = m.up_sql();
            assert!(!up.is_empty(), "Migration {} up_sql must not be empty", m.version);
            assert!(
                up.contains("CREATE TABLE"),
                "Migration {} up_sql should contain CREATE TABLE",
                m.version
            );
        }
    }
}
