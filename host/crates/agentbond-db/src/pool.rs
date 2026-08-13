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

    pub async fn migrations_current(&self) -> Result<bool, DbError> {
        // After migrate(), version table exists. Empty DB before migrate is not current.
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*)::bigint FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name = '_sqlx_migrations'",
        )
        .fetch_one(&self.pool)
        .await?;
        if row.0 == 0 {
            return Ok(false);
        }
        let dirty: (i64,) =
            sqlx::query_as("SELECT COUNT(*)::bigint FROM _sqlx_migrations WHERE success = false")
                .fetch_one(&self.pool)
                .await?;
        Ok(dirty.0 == 0)
    }
}

fn reject_credential_logging(database_url: &str) -> Result<(), DbError> {
    let parsed = Url::parse(database_url).map_err(|e| DbError::Config(e.to_string()))?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        // Credentials are allowed in DATABASE_URL but must never be logged by callers.
        // Reject only clearly malformed schemes.
    }
    if parsed.scheme() != "postgres" && parsed.scheme() != "postgresql" {
        return Err(DbError::Config("DATABASE_URL must be postgres".into()));
    }
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
