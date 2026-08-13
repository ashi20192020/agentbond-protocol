use crate::error::PaymentError;
use crate::models::{PaymentPayload, PaymentRequirements, SCHEME_EXACT, X402_VERSION};

pub fn validate_payment_payload(
    payload: &PaymentPayload,
    expected: &PaymentRequirements,
    now_unix: i64,
    issued_at: i64,
) -> Result<(), PaymentError> {
    if payload.x402_version != X402_VERSION {
        return Err(PaymentError::WrongVersion);
    }
    if !payload.extensions.is_empty() {
        return Err(PaymentError::UnsupportedExtension);
    }
    validate_requirements(&payload.accepted, expected, now_unix, issued_at)?;
    if payload.accepted.scheme != SCHEME_EXACT {
        return Err(PaymentError::WrongScheme);
    }
    if !payload.payload.contains_key("transaction") {
        return Err(PaymentError::InvalidJson);
    }
    Ok(())
}

pub fn validate_requirements(
    actual: &PaymentRequirements,
    expected: &PaymentRequirements,
    now_unix: i64,
    issued_at: i64,
) -> Result<(), PaymentError> {
    if actual.scheme != expected.scheme || actual.scheme != SCHEME_EXACT {
        return Err(PaymentError::WrongScheme);
    }
    if actual.network != expected.network {
        return Err(PaymentError::WrongNetwork);
    }
    if actual.asset != expected.asset {
        return Err(PaymentError::WrongAsset);
    }
    if actual.amount != expected.amount {
        return Err(PaymentError::WrongAmount);
    }
    if actual.pay_to != expected.pay_to {
        return Err(PaymentError::WrongRecipient);
    }
    let expires_at = issued_at
        .checked_add(expected.max_timeout_seconds as i64)
        .ok_or(PaymentError::Expired)?;
    if now_unix > expires_at {
        return Err(PaymentError::Expired);
    }
    Ok(())
}
