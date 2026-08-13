use thiserror::Error;

#[derive(Debug, Error)]
pub enum PaymentError {
    #[error("missing payment")]
    MissingPayment,
    #[error("invalid base64")]
    InvalidBase64,
    #[error("oversized header")]
    OversizedHeader,
    #[error("invalid json")]
    InvalidJson,
    #[error("wrong x402 version")]
    WrongVersion,
    #[error("wrong scheme")]
    WrongScheme,
    #[error("wrong network")]
    WrongNetwork,
    #[error("wrong asset")]
    WrongAsset,
    #[error("wrong amount")]
    WrongAmount,
    #[error("wrong recipient")]
    WrongRecipient,
    #[error("expired requirements")]
    Expired,
    #[error("unsupported extension")]
    UnsupportedExtension,
    #[error("verify rejected")]
    VerifyRejected,
    #[error("verify timeout")]
    VerifyTimeout,
    #[error("settle rejected")]
    SettleRejected,
    #[error("settle timeout")]
    SettleTimeout,
    #[error("facilitator error: {0}")]
    Facilitator(String),
    #[error("invalid config: {0}")]
    Config(String),
}
