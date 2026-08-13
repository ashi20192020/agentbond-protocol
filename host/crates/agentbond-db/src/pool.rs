use std::collections::HashMap;
use std::time::Duration;

use sqlx::postgres::{PgPool, PgPoolOptions};
use url::Url;

use crate::error::DbError;

pub struct Db {
    pool: PgPool,
}

impl Db {
    pub async fn connect(database_url: &str) -> Result<Self, DbError> {
        reject_credential_logging(database_url)?;
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .acquire_timeout(Duration::from_secs(5))
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn migrate(&self) -> Result<(), DbError> {
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }

    pub async fn health(&self) -> Result<(), DbError> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    /// True only when every embedded migration version is present, successful, and checksum-matched.
    pub async fn migrations_current(&self) -> Result<bool, DbError> {
        match self.migrations_status().await {
            Ok(()) => Ok(true),
            Err(DbError::Migration(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Verify embedded migrations are applied with matching checksums.
    pub async fn migrations_status(&self) -> Result<(), DbError> {
        let migrator = sqlx::migrate!("./migrations");
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*)::bigint FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name = '_sqlx_migrations'",
        )
        .fetch_one(&self.pool)
        .await?;
        if row.0 == 0 {
            return Err(DbError::Migration(
                "migrations table missing; run agentbond-indexer migrate".into(),
            ));
        }
        let applied: Vec<(i64, bool, Vec<u8>)> = sqlx::query_as(
            "SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version",
        )
        .fetch_all(&self.pool)
        .await?;
        let applied_map: HashMap<i64, (bool, Vec<u8>)> = applied
            .into_iter()
            .map(|(v, ok, ck)| (v, (ok, ck)))
            .collect();
        let embedded: HashMap<i64, &[u8]> = migrator
            .iter()
            .map(|m| (m.version, m.checksum.as_ref()))
            .collect();
        for version in applied_map.keys() {
            if !embedded.contains_key(version) {
                return Err(DbError::Migration(format!(
                    "unknown applied migration version {version} not present in embedded migrations"
                )));
            }
        }
        for m in migrator.iter() {
            let version = m.version;
            let Some((success, checksum)) = applied_map.get(&version) else {
                return Err(DbError::Migration(format!(
                    "pending migration version {version}; run agentbond-indexer migrate"
                )));
            };
            if !*success {
                return Err(DbError::Migration(format!(
                    "failed migration version {version}; run agentbond-indexer migrate after repair"
                )));
            }
            let expected: &[u8] = m.checksum.as_ref();
            if checksum.as_slice() != expected {
                return Err(DbError::Migration(format!(
                    "checksum mismatch for migration version {version}"
                )));
            }
        }
        Ok(())
    }
}

fn reject_credential_logging(database_url: &str) -> Result<(), DbError> {
    let parsed = Url::parse(database_url).map_err(|e| DbError::Config(e.to_string()))?;
    if parsed.scheme() != "postgres" && parsed.scheme() != "postgresql" {
        return Err(DbError::Config("DATABASE_URL must be postgres".into()));
    }
    let _ = parsed.username();
    Ok(())
}

pub fn redact_db_url(url: &str) -> String {
    match Url::parse(url) {
        Ok(mut u) => {
            let _ = u.set_password(None);
            if !u.username().is_empty() {
                let _ = u.set_username("***");
            }
            u.to_string()
        }
        Err(_) => "<invalid-db-url>".into(),
    }
}
