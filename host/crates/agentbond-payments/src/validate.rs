use base64::Engine;

use crate::challenge::PaymentChallenge;
use crate::error::PaymentError;
use crate::models::{
    MAX_SOLANA_TX_BYTES, PaymentPayload, PaymentRequirements, SCHEME_EXACT, X402_VERSION,
};

pub fn validate_payment_payload(
    payload: &PaymentPayload,
    expected: &PaymentRequirements,
    challenge: &PaymentChallenge,
    now_unix: i64,
    input_digest: &str,
) -> Result<String, PaymentError> {
    if payload.x402_version != X402_VERSION {
        return Err(PaymentError::WrongVersion);
    }
    if !payload.extensions.is_empty() {
        return Err(PaymentError::UnsupportedExtension);
    }
    if payload.resource.url != challenge.resource_url
        || payload.resource.description != challenge.description
        || payload.resource.mime_type != "application/json"
    {
        return Err(PaymentError::InvalidChallenge);
    }
    validate_requirements(&payload.accepted, expected, challenge, now_unix)?;
    if challenge.input_digest != input_digest {
        return Err(PaymentError::BindingMismatch);
    }
    let tx = payload.payload.transaction.trim();
    if tx.is_empty() {
        return Err(PaymentError::InvalidTransaction);
    }
    let decoded = Engine::decode(&base64::engine::general_purpose::STANDARD, tx)
        .map_err(|_| PaymentError::InvalidBase64)?;
    if decoded.is_empty() || decoded.len() > MAX_SOLANA_TX_BYTES {
        return Err(PaymentError::InvalidTransaction);
    }
    Ok(tx.to_string())
}

pub fn validate_requirements(
    actual: &PaymentRequirements,
    expected: &PaymentRequirements,
    challenge: &PaymentChallenge,
    now_unix: i64,
) -> Result<(), PaymentError> {
    if actual.scheme != expected.scheme || actual.scheme != SCHEME_EXACT {
        return Err(PaymentError::WrongScheme);
    }
    if actual.network != expected.network || actual.network != challenge.network {
        return Err(PaymentError::WrongNetwork);
    }
    if actual.asset != expected.asset || actual.asset != challenge.asset {
        return Err(PaymentError::WrongAsset);
    }
    if actual.amount != expected.amount || actual.amount != challenge.amount {
        return Err(PaymentError::WrongAmount);
    }
    if actual.pay_to != expected.pay_to || actual.pay_to != challenge.merchant {
        return Err(PaymentError::WrongRecipient);
    }
    if actual.max_timeout_seconds != expected.max_timeout_seconds {
        return Err(PaymentError::InvalidChallenge);
    }
    if actual.extra.fee_payer != expected.extra.fee_payer
        || actual.extra.fee_payer != challenge.fee_payer
    {
        return Err(PaymentError::WrongFeePayer);
    }
    let memo = actual
        .extra
        .memo
        .as_deref()
        .ok_or(PaymentError::InvalidChallenge)?;
    if memo != challenge.memo || memo.len() < 32 {
        return Err(PaymentError::InvalidChallenge);
    }
    if actual
        .extra
        .recent_blockhash
        .as_deref()
        .is_some_and(|bh| bh.is_empty() || bh.len() > 88)
    {
        return Err(PaymentError::InvalidChallenge);
    }
    if actual.extra.last_valid_block_height.is_some() && actual.extra.recent_blockhash.is_none() {
        return Err(PaymentError::InvalidChallenge);
    }
    let expires_at = challenge
        .issued_at
        .checked_add(challenge.max_timeout_seconds as i64)
        .ok_or(PaymentError::Expired)?;
    if now_unix > expires_at {
        return Err(PaymentError::Expired);
    }
    Ok(())
}
