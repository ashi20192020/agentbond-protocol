use thiserror::Error;

#[derive(Debug, Error)]
pub enum IndexerError {
    #[error(transparent)]
    Db(#[from] agentbond_db::DbError),
    #[error("config: {0}")]
    Config(String),
    #[error("decode: {0}")]
    Decode(String),
    #[error("source: {0}")]
    Source(String),
    #[error("fixture: {0}")]
    Fixture(String),
    #[error("backfill: {0}")]
    Backfill(String),
}
