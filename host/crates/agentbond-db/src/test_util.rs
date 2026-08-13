use std::fs::File;
use std::sync::Arc;

use fs2::FileExt;

use crate::error::DbError;
use crate::pool::Db;

/// Serialize PostgreSQL integration tests that share one local database.
pub fn pg_test_lock() -> Result<File, DbError> {
    let path = std::env::temp_dir().join("agentbond-pg-test.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| DbError::Config(format!("pg test lock open: {e}")))?;
    file.lock_exclusive()
        .map_err(|e| DbError::Config(format!("pg test lock: {e}")))?;
    Ok(file)
}

pub async fn connect_migrated(database_url: &str) -> Result<Arc<Db>, DbError> {
    let db = Arc::new(Db::connect(database_url).await?);
    db.migrate().await?;
    Ok(db)
}

pub async fn reset_public_tables(db: &Db) -> Result<(), DbError> {
    sqlx::query(
        "TRUNCATE TABLE
            x402_settlements,
            x402_challenges,
            proj_challenges,
            proj_jobs,
            proj_provider_bonds,
            proj_providers,
            proj_config,
            raw_protocol_events,
            raw_account_versions,
            ingestion_gaps,
            indexer_slots
         RESTART IDENTITY CASCADE",
    )
    .execute(db.pool())
    .await?;
    sqlx::query(
        "UPDATE indexer_checkpoints
         SET finalized_slot = 0, processed_slot = 0, yellowstone_resume = NULL, updated_at = NOW()
         WHERE id = 1",
    )
    .execute(db.pool())
    .await?;
    Ok(())
}
