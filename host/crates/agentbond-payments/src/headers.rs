use base64::Engine;
use serde::Serialize;

use crate::error::PaymentError;
use crate::models::{PaymentPayload, PaymentRequired, SettleResponse};

pub const PAYMENT_REQUIRED: &str = "PAYMENT-REQUIRED";
pub const PAYMENT_SIGNATURE: &str = "PAYMENT-SIGNATURE";
pub const PAYMENT_RESPONSE: &str = "PAYMENT-RESPONSE";

pub const MAX_HEADER_BYTES: usize = 16 * 1024;

fn decode_b64_json<T: for<'de> serde::Deserialize<'de>>(value: &str) -> Result<T, PaymentError> {
    if value.len() > MAX_HEADER_BYTES {
        return Err(PaymentError::OversizedHeader);
    }
    let bytes = Engine::decode(&base64::engine::general_purpose::STANDARD, value.trim())
        .map_err(|_| PaymentError::InvalidBase64)?;
    if bytes.len() > MAX_HEADER_BYTES {
        return Err(PaymentError::OversizedHeader);
    }
    serde_json::from_slice(&bytes).map_err(|_| PaymentError::InvalidJson)
}

fn encode_b64_json<T: Serialize>(value: &T) -> Result<String, PaymentError> {
    let json = serde_json::to_vec(value).map_err(|_| PaymentError::InvalidJson)?;
    if json.len() > MAX_HEADER_BYTES {
        return Err(PaymentError::OversizedHeader);
    }
    Ok(Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        json,
    ))
}

pub fn encode_payment_required_header(value: &PaymentRequired) -> Result<String, PaymentError> {
    encode_b64_json(value)
}

pub fn decode_payment_signature_header(value: &str) -> Result<PaymentPayload, PaymentError> {
    decode_b64_json(value)
}

pub fn encode_payment_response_header(value: &SettleResponse) -> Result<String, PaymentError> {
    encode_b64_json(value)
}

/// Headers that must never appear in logs.
pub fn is_sensitive_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "payment-signature" | "payment-required" | "payment-response" | "authorization"
    )
}
