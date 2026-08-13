use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("database: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("migrate: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("config: {0}")]
    Config(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("validation: {0}")]
    Validation(String),
    #[error(transparent)]
    Payment(#[from] agentbond_payments::PaymentError),
}
