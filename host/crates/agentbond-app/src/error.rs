use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    Validation(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Sdk(#[from] agentbond_sdk::SdkError),
    #[error("config: {0}")]
    Config(String),
}
