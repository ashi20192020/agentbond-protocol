use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::error::PaymentError;
use crate::facilitator::FacilitatorClient;
use crate::headers::{
    decode_payment_signature_header, encode_payment_required_header, encode_payment_response_header,
};
use crate::models::{
    PaymentRequired, PaymentRequirements, ResourceInfo, SCHEME_EXACT, SettleRequest,
    SettleResponse, VerifyRequest, X402_VERSION,
};
use crate::validate::validate_payment_payload;

#[derive(Clone, Debug)]
pub struct X402ResourceConfig {
    pub network: String,
    pub asset: String,
    pub pay_to: String,
    pub amount: String,
    pub max_timeout_seconds: u64,
    pub resource_url: String,
    pub description: String,
}

#[derive(Clone, Debug)]
pub struct PaidDemoResult {
    pub body: serde_json::Value,
    pub payment_response_header: String,
}

#[derive(Default)]
pub struct PaymentCache {
    inner: Mutex<HashMap<String, PaidDemoResult>>,
}

impl PaymentCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get(&self, key: &str) -> Option<PaidDemoResult> {
        self.inner.lock().await.get(key).cloned()
    }

    pub async fn insert(&self, key: String, value: PaidDemoResult) {
        let mut guard = self.inner.lock().await;
        if guard.len() >= 256 {
            guard.clear();
        }
        guard.insert(key, value);
    }
}

pub fn build_payment_required(
    cfg: &X402ResourceConfig,
) -> Result<(PaymentRequired, PaymentRequirements, String), PaymentError> {
    let requirements = PaymentRequirements {
        scheme: SCHEME_EXACT.into(),
        network: cfg.network.clone(),
        amount: cfg.amount.clone(),
        asset: cfg.asset.clone(),
        pay_to: cfg.pay_to.clone(),
        max_timeout_seconds: cfg.max_timeout_seconds,
        extra: None,
    };
    let required = PaymentRequired {
        x402_version: X402_VERSION,
        error: Some("PAYMENT-SIGNATURE header is required".into()),
        resource: ResourceInfo {
            url: cfg.resource_url.clone(),
            description: cfg.description.clone(),
            mime_type: "application/json".into(),
        },
        accepts: vec![requirements.clone()],
    };
    let header = encode_payment_required_header(&required)?;
    Ok((required, requirements, header))
}

pub async fn invoke_paid_demo(
    cfg: &X402ResourceConfig,
    facilitator: &dyn FacilitatorClient,
    cache: &PaymentCache,
    payment_header: Option<&str>,
    input: &serde_json::Value,
    now_unix: i64,
    issued_at: i64,
) -> Result<Result<PaidDemoResult, String>, PaymentError> {
    let (_required, requirements, payment_required_header) = build_payment_required(cfg)?;
    let Some(header) = payment_header else {
        return Ok(Err(payment_required_header));
    };

    if let Some(cached) = cache.get(header).await {
        return Ok(Ok(cached));
    }

    let payload = decode_payment_signature_header(header)?;
    validate_payment_payload(&payload, &requirements, now_unix, issued_at)?;

    let verify = facilitator
        .verify(&VerifyRequest {
            x402_version: X402_VERSION,
            payment_payload: payload.clone(),
            payment_requirements: requirements.clone(),
        })
        .await?;
    if !verify.is_valid {
        return Err(PaymentError::VerifyRejected);
    }

    // Deterministic transform only — not an AI model.
    let input_bytes = serde_json::to_vec(input).map_err(|_| PaymentError::InvalidJson)?;
    let digest = Sha256::digest(input_bytes);
    let body = serde_json::json!({
        "service": "agentbond-x402-demo",
        "input_sha256": hex::encode(digest),
        "echo": input,
        "note": "deterministic paid demo resource",
    });

    let settle = facilitator
        .settle(&SettleRequest {
            x402_version: X402_VERSION,
            payment_payload: payload,
            payment_requirements: requirements,
        })
        .await?;
    if !settle.success {
        return Err(PaymentError::SettleRejected);
    }

    let response_header = encode_payment_response_header(&settle)?;
    let result = PaidDemoResult {
        body,
        payment_response_header: response_header,
    };
    cache.insert(header.to_string(), result.clone()).await;
    Ok(Ok(result))
}

// Avoid pulling hex workspace dep into payments — use simple encode.
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let bytes = bytes.as_ref();
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0xf) as usize] as char);
        }
        out
    }
}

pub fn settle_response_ok() -> SettleResponse {
    SettleResponse {
        success: true,
        error_reason: None,
        transaction: Some("cached".into()),
        network: None,
        payer: None,
    }
}

pub type SharedFacilitator = Arc<dyn FacilitatorClient>;
