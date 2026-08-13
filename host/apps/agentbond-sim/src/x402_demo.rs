//! Local x402 402 → verify → settle → 200 using MockFacilitatorClient.
//! The paid resource is a deterministic hash echo — not an AI model.

use agentbond_payments::{
    ChallengeStore, ExactPayloadBody, MockFacilitatorClient, PAYMENT_REQUIRED, PaymentPayload,
    ResourceInfo, SCHEME_EXACT, SettlementStore, SvmExactExtra, X402_VERSION, X402ResourceConfig,
    invoke_paid_demo,
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

pub async fn run_x402_demo() -> Result<X402DemoOutcome> {
    let cfg = X402ResourceConfig {
        network: "solana:localnet".into(),
        asset: "So11111111111111111111111111111111111111112".into(),
        pay_to: "11111111111111111111111111111112".into(),
        fee_payer: "11111111111111111111111111111113".into(),
        amount: "1000".into(),
        max_timeout_seconds: 60,
        resource_url: "/v1/x402/services/hash-demo/invoke".into(),
        description: "deterministic paid hash-demo resource".into(),
        service_id: "hash-demo".into(),
    };
    let facilitator = MockFacilitatorClient::new();
    let challenges = ChallengeStore::new();
    let settlements = SettlementStore::new();
    let input = json!({"ping": "local-sim"});
    let now = 1_700_000_000_i64;

    println!("  {PAYMENT_REQUIRED}: missing PAYMENT-SIGNATURE → 402");
    let first = invoke_paid_demo(
        &cfg,
        &facilitator,
        &challenges,
        &settlements,
        None,
        &input,
        now,
    )
    .await
    .map_err(|e| anyhow!("x402 without payment: {e}"))?;
    let Err(payment_required_header) = first else {
        bail!("expected 402 payment-required when signature header is absent");
    };

    let required: agentbond_payments::PaymentRequired = {
        let bytes = Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            payment_required_header.trim(),
        )
        .map_err(|e| anyhow!("decode payment-required: {e}"))?;
        serde_json::from_slice(&bytes).map_err(|e| anyhow!("parse payment-required: {e}"))?
    };
    let accepted = required
        .accepts
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("missing accepts"))?;
    let memo = accepted
        .extra
        .memo
        .clone()
        .ok_or_else(|| anyhow!("missing feePayer memo"))?;

    println!("  PAYMENT-SIGNATURE present → mock verify → mock settle → 200");
    let payment = PaymentPayload {
        x402_version: X402_VERSION,
        resource: ResourceInfo {
            url: cfg.resource_url.clone(),
            description: cfg.description.clone(),
            mime_type: "application/json".into(),
        },
        accepted: agentbond_payments::PaymentRequirements {
            scheme: SCHEME_EXACT.into(),
            network: cfg.network.clone(),
            amount: cfg.amount.clone(),
            asset: cfg.asset.clone(),
            pay_to: cfg.pay_to.clone(),
            max_timeout_seconds: cfg.max_timeout_seconds,
            extra: SvmExactExtra {
                fee_payer: cfg.fee_payer.clone(),
                memo: Some(memo),
                recent_blockhash: None,
                last_valid_block_height: None,
            },
        },
        payload: ExactPayloadBody {
            transaction: Engine::encode(&base64::engine::general_purpose::STANDARD, [1u8; 64]),
        },
        extensions: Default::default(),
    };
    let header = Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        serde_json::to_vec(&payment)?,
    );

    let second = invoke_paid_demo(
        &cfg,
        &facilitator,
        &challenges,
        &settlements,
        Some(&header),
        &input,
        now,
    )
    .await
    .map_err(|e| anyhow!("x402 with payment: {e}"))?;
    let Ok(paid) = second else {
        bail!("expected 200 paid demo result after mock verify/settle");
    };

    if paid.body.get("service").and_then(|v| v.as_str()) != Some("agentbond-x402-demo") {
        bail!("unexpected demo body: {}", paid.body);
    }

    Ok(X402DemoOutcome {
        status_without_payment: 402,
        status_with_payment: 200,
        body: paid.body,
        payment_response_header: paid.payment_response_header,
    })
}
