//! Narrow x402 v2 resource-server adapter (exact scheme on Solana).
//! Not an official or complete Rust x402 SDK.

pub mod challenge;
pub mod error;
pub mod facilitator;
pub mod headers;
pub mod http_util;
pub mod models;
pub mod resource;
pub mod settlement;
pub mod validate;

pub use challenge::ChallengeStore;
pub use error::PaymentError;
pub use facilitator::{FacilitatorClient, HttpFacilitatorClient, MockFacilitatorClient};
pub use headers::{
    MAX_HEADER_BYTES, PAYMENT_REQUIRED, PAYMENT_RESPONSE, PAYMENT_SIGNATURE,
    decode_payment_signature_header, encode_payment_required_header,
    encode_payment_response_header, is_sensitive_header,
};
pub use models::*;
pub use resource::{PaidDemoResult, X402ResourceConfig, input_digest, invoke_paid_demo};
pub use settlement::{SettlementBinding, SettlementStore};
pub use validate::validate_payment_payload;
