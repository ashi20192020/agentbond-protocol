use std::sync::Arc;

use agentbond_payments::{
    ChallengeStore, ExactPayloadBody, MAX_HEADER_BYTES, MockFacilitatorClient, PaymentPayload,
    PaymentRequired, ResourceInfo, SettlementStore, SvmExactExtra, X402_VERSION,
    X402ResourceConfig, decode_payment_signature_header, encode_payment_required_header,
    invoke_paid_demo, is_sensitive_header,
};
use base64::Engine;
use serde_json::json;

fn cfg() -> X402ResourceConfig {
    X402ResourceConfig {
        network: "solana:localnet".into(),
        asset: "11111111111111111111111111111111".into(),
        pay_to: "11111111111111111111111111111112".into(),
        fee_payer: "11111111111111111111111111111113".into(),
        amount: "1000".into(),
        max_timeout_seconds: 60,
        resource_url: "/v1/x402/services/hash-demo/invoke".into(),
        description: "demo".into(),
        service_id: "hash-demo".into(),
    }
}

fn b64(v: &impl serde::Serialize) -> String {
    Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        serde_json::to_vec(v).expect("json"),
    )
}

fn tx_b64() -> String {
    Engine::encode(&base64::engine::general_purpose::STANDARD, [7u8; 64])
}

async fn issue_header(
    cfg: &X402ResourceConfig,
    challenges: &ChallengeStore,
    settlements: &SettlementStore,
    fac: &MockFacilitatorClient,
    input: &serde_json::Value,
    now: i64,
) -> (String, PaymentPayload) {
    let first = invoke_paid_demo(cfg, fac, challenges, settlements, None, input, now)
        .await
        .expect("issue");
    let Err(header) = first else {
        panic!("expected payment required");
    };
    let required: PaymentRequired = {
        let bytes =
            Engine::decode(&base64::engine::general_purpose::STANDARD, header.trim()).expect("b64");
        serde_json::from_slice(&bytes).expect("json")
    };
    let accepted = required.accepts[0].clone();
    let payload = PaymentPayload {
        x402_version: X402_VERSION,
        resource: ResourceInfo {
            url: cfg.resource_url.clone(),
            description: cfg.description.clone(),
            mime_type: "application/json".into(),
        },
        accepted,
        payload: ExactPayloadBody {
            transaction: tx_b64(),
        },
        extensions: Default::default(),
    };
    (b64(&payload), payload)
}

#[tokio::test]
async fn missing_payment_returns_402_header() {
    let fac = MockFacilitatorClient::new();
    let challenges = ChallengeStore::new();
    let settlements = SettlementStore::new();
    let out = invoke_paid_demo(&cfg(), &fac, &challenges, &settlements, None, &json!({}), 1)
        .await
        .expect("ok");
    assert!(out.is_err());
}

#[tokio::test]
async fn invalid_base64_and_oversized_header() {
    assert!(decode_payment_signature_header("%%%").is_err());
    let huge = "A".repeat(MAX_HEADER_BYTES + 1);
    assert!(decode_payment_signature_header(&huge).is_err());
}

#[tokio::test]
async fn wrong_fields_rejected() {
    let fac = MockFacilitatorClient::new();
    let challenges = ChallengeStore::new();
    let settlements = SettlementStore::new();
    let c = cfg();
    let input = json!({"a":1});
    let (header, mut payload) =
        issue_header(&c, &challenges, &settlements, &fac, &input, 100).await;
    let _ = header;
    payload.x402_version = 1;
    let bad = b64(&payload);
    let err = invoke_paid_demo(&c, &fac, &challenges, &settlements, Some(&bad), &input, 100)
        .await
        .expect_err("version");
    assert!(err.to_string().contains("version") || err.to_string().contains("Wrong"));

    let (_, mut payload) = issue_header(&c, &challenges, &settlements, &fac, &input, 101).await;
    payload.accepted.scheme = "upto".into();
    let bad = b64(&payload);
    assert!(
        invoke_paid_demo(&c, &fac, &challenges, &settlements, Some(&bad), &input, 101)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn successful_payment_and_exact_retry() {
    let fac = MockFacilitatorClient::new();
    let challenges = ChallengeStore::new();
    let settlements = SettlementStore::new();
    let c = cfg();
    let input = json!({"ping":1});
    let (header, _) = issue_header(&c, &challenges, &settlements, &fac, &input, 200).await;
    let first = invoke_paid_demo(
        &c,
        &fac,
        &challenges,
        &settlements,
        Some(&header),
        &input,
        200,
    )
    .await
    .expect("pay")
    .expect("200");
    assert_eq!(fac.settle_calls().await, 1);
    let second = invoke_paid_demo(
        &c,
        &fac,
        &challenges,
        &settlements,
        Some(&header),
        &input,
        200,
    )
    .await
    .expect("retry")
    .expect("cached");
    assert_eq!(fac.settle_calls().await, 1);
    assert_eq!(first.body, second.body);
}

#[tokio::test]
async fn concurrent_settle_once() {
    let fac = Arc::new(MockFacilitatorClient::new());
    let challenges = Arc::new(ChallengeStore::new());
    let settlements = Arc::new(SettlementStore::new());
    let c = cfg();
    let input = json!({"c":1});
    let (header, _) = issue_header(&c, &challenges, &settlements, &fac, &input, 300).await;
    let header = Arc::new(header);
    let mut joins = Vec::new();
    for _ in 0..8 {
        let fac = fac.clone();
        let challenges = challenges.clone();
        let settlements = settlements.clone();
        let header = header.clone();
        let c = c.clone();
        let input = input.clone();
        joins.push(tokio::spawn(async move {
            invoke_paid_demo(
                &c,
                fac.as_ref(),
                challenges.as_ref(),
                settlements.as_ref(),
                Some(header.as_str()),
                &input,
                300,
            )
            .await
        }));
    }
    let mut oks = 0u32;
    let mut in_progress = 0u32;
    for j in joins {
        match j.await.expect("join") {
            Ok(Ok(_)) => oks += 1,
            Err(e) if e.to_string().contains("in progress") || e.to_string().contains("retry") => {
                in_progress += 1
            }
            Ok(Err(_)) => {}
            Err(_) => {}
        }
    }
    assert_eq!(fac.settle_calls().await, 1);
    assert!(oks >= 1, "at least one success");
    let _ = in_progress;
}

#[tokio::test]
async fn verify_and_settle_failures() {
    let fac = MockFacilitatorClient::new();
    fac.set_verify_ok(false).await;
    let challenges = ChallengeStore::new();
    let settlements = SettlementStore::new();
    let c = cfg();
    let input = json!({});
    let (header, _) = issue_header(&c, &challenges, &settlements, &fac, &input, 400).await;
    assert!(
        invoke_paid_demo(
            &c,
            &fac,
            &challenges,
            &settlements,
            Some(&header),
            &input,
            400
        )
        .await
        .is_err()
    );

    let fac = MockFacilitatorClient::new();
    fac.set_settle_ok(false).await;
    let challenges = ChallengeStore::new();
    let settlements = SettlementStore::new();
    let (header, _) = issue_header(&c, &challenges, &settlements, &fac, &input, 401).await;
    assert!(
        invoke_paid_demo(
            &c,
            &fac,
            &challenges,
            &settlements,
            Some(&header),
            &input,
            401
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn sensitive_header_redaction_helper() {
    assert!(is_sensitive_header("PAYMENT-SIGNATURE"));
    assert!(is_sensitive_header("Authorization"));
    assert!(!is_sensitive_header("content-type"));
}

#[tokio::test]
async fn payment_required_encodes_fee_payer() {
    let fac = MockFacilitatorClient::new();
    let challenges = ChallengeStore::new();
    let settlements = SettlementStore::new();
    let c = cfg();
    let Err(header) = invoke_paid_demo(&c, &fac, &challenges, &settlements, None, &json!({}), 1)
        .await
        .expect("ok")
    else {
        panic!("expected header");
    };
    let required: PaymentRequired = {
        let bytes =
            Engine::decode(&base64::engine::general_purpose::STANDARD, header.trim()).expect("b64");
        serde_json::from_slice(&bytes).expect("json")
    };
    assert_eq!(required.accepts[0].extra.fee_payer, c.fee_payer);
    assert!(required.accepts[0].extra.memo.as_ref().unwrap().len() >= 32);
    let _ = encode_payment_required_header(&required).expect("encode");
    let _ = SvmExactExtra {
        fee_payer: c.fee_payer,
        memo: None,
        recent_blockhash: None,
        last_valid_block_height: None,
    };
}
