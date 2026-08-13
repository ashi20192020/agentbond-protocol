//! Milestone 3 payment adapter coverage.

use std::collections::BTreeMap;
use std::time::Duration;

use agentbond_payments::{
    MAX_HEADER_BYTES, MockFacilitatorClient, PAYMENT_REQUIRED, PAYMENT_RESPONSE, PAYMENT_SIGNATURE,
    PaymentCache, PaymentError, PaymentPayload, PaymentRequirements, ResourceInfo, SCHEME_EXACT,
    X402_VERSION, X402ResourceConfig, build_payment_required, decode_payment_signature_header,
    invoke_paid_demo, is_sensitive_header, validate_payment_payload,
};
use base64::Engine;
use serde_json::json;

fn cfg() -> X402ResourceConfig {
    X402ResourceConfig {
        network: "solana:localnet".into(),
        asset: "11111111111111111111111111111111".into(),
        pay_to: "11111111111111111111111111111112".into(),
        amount: "1000".into(),
        max_timeout_seconds: 60,
        resource_url: "/v1/x402/services/hash-demo/invoke".into(),
        description: "demo".into(),
    }
}

fn requirements(cfg: &X402ResourceConfig) -> PaymentRequirements {
    PaymentRequirements {
        scheme: SCHEME_EXACT.into(),
        network: cfg.network.clone(),
        amount: cfg.amount.clone(),
        asset: cfg.asset.clone(),
        pay_to: cfg.pay_to.clone(),
        max_timeout_seconds: cfg.max_timeout_seconds,
        extra: None,
    }
}

fn valid_payload(cfg: &X402ResourceConfig) -> PaymentPayload {
    let mut payload = BTreeMap::new();
    payload.insert("transaction".into(), json!("deadbeef"));
    PaymentPayload {
        x402_version: X402_VERSION,
        resource: ResourceInfo {
            url: cfg.resource_url.clone(),
            description: cfg.description.clone(),
            mime_type: "application/json".into(),
        },
        accepted: requirements(cfg),
        payload,
        extensions: BTreeMap::new(),
    }
}

fn encode_header(payload: &PaymentPayload) -> String {
    Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        serde_json::to_vec(payload).expect("json"),
    )
}

#[tokio::test]
async fn missing_payment_returns_payment_required_header() {
    let cfg = cfg();
    let fac = MockFacilitatorClient::new();
    let cache = PaymentCache::new();
    let result = invoke_paid_demo(
        &cfg,
        &fac,
        &cache,
        None,
        &json!({"n": 1}),
        1_700_000_000,
        1_700_000_000,
    )
    .await
    .expect("no hard error");
    let Err(header) = result else {
        panic!("expected missing payment path");
    };
    assert!(!header.is_empty());
    let decoded = build_payment_required(&cfg).expect("build").0;
    assert_eq!(decoded.x402_version, X402_VERSION);
}

#[test]
fn invalid_base64_and_oversized_header() {
    assert!(matches!(
        decode_payment_signature_header("%%%not-base64%%%"),
        Err(PaymentError::InvalidBase64)
    ));

    let oversized = "A".repeat(MAX_HEADER_BYTES + 1);
    assert!(matches!(
        decode_payment_signature_header(&oversized),
        Err(PaymentError::OversizedHeader)
    ));
}

#[test]
fn wrong_version_scheme_network_asset_amount_recipient() {
    let cfg = cfg();
    let expected = requirements(&cfg);
    let now = 1_700_000_000i64;
    let issued = now;

    let mut payload = valid_payload(&cfg);
    payload.x402_version = 1;
    assert!(matches!(
        validate_payment_payload(&payload, &expected, now, issued),
        Err(PaymentError::WrongVersion)
    ));

    payload = valid_payload(&cfg);
    payload.accepted.scheme = "upto".into();
    assert!(matches!(
        validate_payment_payload(&payload, &expected, now, issued),
        Err(PaymentError::WrongScheme)
    ));

    payload = valid_payload(&cfg);
    payload.accepted.network = "solana:mainnet".into();
    assert!(matches!(
        validate_payment_payload(&payload, &expected, now, issued),
        Err(PaymentError::WrongNetwork)
    ));

    payload = valid_payload(&cfg);
    payload.accepted.asset = "DifferentMint1111111111111111111111111111".into();
    assert!(matches!(
        validate_payment_payload(&payload, &expected, now, issued),
        Err(PaymentError::WrongAsset)
    ));

    payload = valid_payload(&cfg);
    payload.accepted.amount = "9999".into();
    assert!(matches!(
        validate_payment_payload(&payload, &expected, now, issued),
        Err(PaymentError::WrongAmount)
    ));

    payload = valid_payload(&cfg);
    payload.accepted.pay_to = "11111111111111111111111111111113".into();
    assert!(matches!(
        validate_payment_payload(&payload, &expected, now, issued),
        Err(PaymentError::WrongRecipient)
    ));
}

#[tokio::test]
async fn verify_rejected_and_timeout() {
    let cfg = cfg();
    let cache = PaymentCache::new();
    let header = encode_header(&valid_payload(&cfg));
    let now = 1_700_000_000i64;

    let fac = MockFacilitatorClient::new();
    fac.set_verify_ok(false).await;
    let err = invoke_paid_demo(
        &cfg,
        &fac,
        &cache,
        Some(&header),
        &json!({"x": 1}),
        now,
        now,
    )
    .await
    .expect_err("verify rejected");
    assert!(matches!(err, PaymentError::VerifyRejected));

    let fac = MockFacilitatorClient::new();
    fac.set_verify_delay(Some(Duration::from_millis(1))).await;
    let err = invoke_paid_demo(
        &cfg,
        &fac,
        &cache,
        Some(&header),
        &json!({"x": 1}),
        now,
        now,
    )
    .await
    .expect_err("verify timeout");
    assert!(matches!(err, PaymentError::VerifyTimeout));
}

#[tokio::test]
async fn settle_rejected_and_timeout() {
    let cfg = cfg();
    let cache = PaymentCache::new();
    let header = encode_header(&valid_payload(&cfg));
    let now = 1_700_000_000i64;

    let fac = MockFacilitatorClient::new();
    fac.set_settle_ok(false).await;
    let err = invoke_paid_demo(
        &cfg,
        &fac,
        &cache,
        Some(&header),
        &json!({"x": 1}),
        now,
        now,
    )
    .await
    .expect_err("settle rejected");
    assert!(matches!(err, PaymentError::SettleRejected));

    let fac = MockFacilitatorClient::new();
    fac.set_settle_delay(Some(Duration::from_millis(1))).await;
    let err = invoke_paid_demo(
        &cfg,
        &fac,
        &cache,
        Some(&header),
        &json!({"x": 1}),
        now,
        now,
    )
    .await
    .expect_err("settle timeout");
    assert!(matches!(err, PaymentError::SettleTimeout));
}

#[tokio::test]
async fn successful_payment_and_retry_cache() {
    let cfg = cfg();
    let fac = MockFacilitatorClient::new();
    let cache = PaymentCache::new();
    let header = encode_header(&valid_payload(&cfg));
    let now = 1_700_000_000i64;
    let input = json!({"hello": "world"});

    let first = invoke_paid_demo(&cfg, &fac, &cache, Some(&header), &input, now, now)
        .await
        .expect("ok")
        .expect("paid");
    assert_eq!(first.body["service"], "agentbond-x402-demo");
    assert!(!first.payment_response_header.is_empty());

    // Force facilitator to reject; cached retry must still succeed.
    fac.set_verify_ok(false).await;
    fac.set_settle_ok(false).await;
    let second = invoke_paid_demo(&cfg, &fac, &cache, Some(&header), &input, now, now)
        .await
        .expect("ok")
        .expect("cached");
    assert_eq!(second.body, first.body);
    assert_eq!(
        second.payment_response_header,
        first.payment_response_header
    );
}

#[test]
fn sensitive_header_redaction() {
    assert!(is_sensitive_header(PAYMENT_SIGNATURE));
    assert!(is_sensitive_header(PAYMENT_REQUIRED));
    assert!(is_sensitive_header(PAYMENT_RESPONSE));
    assert!(is_sensitive_header("Authorization"));
    assert!(is_sensitive_header("payment-signature"));
    assert!(!is_sensitive_header("content-type"));
    assert!(!is_sensitive_header("x-request-id"));
}
