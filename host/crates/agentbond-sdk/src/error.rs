use thiserror::Error;

#[derive(Debug, Error)]
pub enum SdkError {
    #[error("invalid pubkey: {0}")]
    InvalidPubkey(String),
    #[error("invalid amount")]
    InvalidAmount,
    #[error("invalid deadline order")]
    InvalidDeadlineOrder,
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("wrong account owner")]
    WrongOwner,
    #[error("wrong account address")]
    WrongAddress,
    #[error("account decode failed: {0}")]
    Decode(String),
    #[error("rpc error: {0}")]
    Rpc(String),
    #[error("serialization error: {0}")]
    Serde(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

impl From<agentbond_types::ProtocolError> for SdkError {
    fn from(value: agentbond_types::ProtocolError) -> Self {
        Self::Protocol(value.as_str().to_string())
    }
}

impl From<serde_json::Error> for SdkError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serde(value.to_string())
    }
}
