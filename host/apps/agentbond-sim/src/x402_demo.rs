//! Local x402 402 → verify → settle → 200 using MockFacilitatorClient.
//! The paid resource is a deterministic hash echo — not an AI model.

use std::collections::BTreeMap;

use agentbond_payments::{
    MockFacilitatorClient, PAYMENT_REQUIRED, PaymentCache, PaymentPayload, PaymentRequirements,
    ResourceInfo, SCHEME_EXACT, X402_VERSION, X402ResourceConfig, invoke_paid_demo,
};
use anyhow::{Result, anyhow, bail};
use base64::Engine;
use serde_json::json;

pub struct X402DemoOutcome {
    pub status_without_payment: u16,
    pub status_with_payment: u16,
    pub body: serde_json::Value,
    pub payment_response_header: String,
}

fn encode_payment_signature_header(payload: &PaymentPayload) -> Result<String> {
    let json = serde_json::to_vec(payload).map_err(|e| anyhow!("payment payload json: {e}"))?;
    Ok(Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        json,
    ))
}

fn mock_payment_header(cfg: &X402ResourceConfig) -> Result<String> {
    let accepted = PaymentRequirements {
        scheme: SCHEME_EXACT.into(),
        network: cfg.network.clone(),
        amount: cfg.amount.clone(),
        asset: cfg.asset.clone(),
        pay_to: cfg.pay_to.clone(),
        max_timeout_seconds: cfg.max_timeout_seconds,
        extra: None,
    };
    let mut payload_map = BTreeMap::new();
    payload_map.insert("transaction".into(), json!("mock-local-tx"));
    let payment = PaymentPayload {
        x402_version: X402_VERSION,
        resource: ResourceInfo {
            url: cfg.resource_url.clone(),
            description: cfg.description.clone(),
            mime_type: "application/json".into(),
        },
        accepted,
        payload: payload_map,
        extensions: BTreeMap::new(),
    };
    encode_payment_signature_header(&payment)
}

pub async fn run_x402_demo() -> Result<X402DemoOutcome> {
    let cfg = X402ResourceConfig {
        network: "solana:localnet".into(),
        asset: "So11111111111111111111111111111111111111112".into(),
        pay_to: "DemoMerchant1111111111111111111111111111111".into(),
        amount: "1000".into(),
        max_timeout_seconds: 60,
        resource_url: "/v1/x402/services/hash-demo/invoke".into(),
        description: "deterministic paid hash-demo resource".into(),
    };
    let facilitator = MockFacilitatorClient::new();
    let cache = PaymentCache::new();
    let input = json!({"ping": "local-sim"});
    let now = 1_700_000_000_i64;
    let issued_at = now;

    println!("  {PAYMENT_REQUIRED}: missing PAYMENT-SIGNATURE → 402");
    let first = invoke_paid_demo(&cfg, &facilitator, &cache, None, &input, now, issued_at)
        .await
        .map_err(|e| anyhow!("x402 without payment: {e}"))?;
    let Err(_payment_required_header) = first else {
        bail!("expected 402 payment-required when signature header is absent");
    };

    println!("  PAYMENT-SIGNATURE present → mock verify → mock settle → 200");
    let header = mock_payment_header(&cfg)?;
    let second = invoke_paid_demo(
        &cfg,
        &facilitator,
        &cache,
        Some(&header),
        &input,
        now,
        issued_at,
    )
    .await
    .map_err(|e| anyhow!("x402 with payment: {e}"))?;
    let Ok(paid) = second else {
        bail!("expected 200 paid demo result after mock verify/settle");
    };

    if paid.body.get("service").and_then(|v| v.as_str()) != Some("agentbond-x402-demo") {
        bail!("unexpected demo body: {}", paid.body);
    }
    if paid.body.get("note").and_then(|v| v.as_str()) != Some("deterministic paid demo resource") {
        bail!("demo body must describe a deterministic resource, not an AI model");
    }

    Ok(X402DemoOutcome {
        status_without_payment: 402,
        status_with_payment: 200,
        body: paid.body,
        payment_response_header: paid.payment_response_header,
    })
}
