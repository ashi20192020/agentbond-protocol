//! Milestone 3 gateway route tests via axum Router + tower ServiceExt.

use std::sync::Arc;
use std::time::Duration;

use agentbond_app::{AppConfig, ServiceCatalog, ServiceEntry};
use agentbond_gateway::{router, test_state};
use agentbond_payments::MockFacilitatorClient;
use agentbond_payments::headers::PAYMENT_SIGNATURE;
use agentbond_sdk::{AccountData, ChainReader, MockChainReader, job_pda, program_id, provider_pda};
use agentbond_types::{JobAccount, JobState, PROVIDER_STATUS_ACTIVE, ProviderAccount};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use solana_pubkey::Pubkey;
use tower::ServiceExt;

const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

fn test_config() -> AppConfig {
    AppConfig {
        program_id: program_id().to_string(),
        rpc_url: "http://127.0.0.1:8899".into(),
        genesis_hash: "07".repeat(32),
        settlement_mint: "11111111111111111111111111111111".into(),
        token_program: TOKEN_PROGRAM.into(),
        facilitator_url: "http://127.0.0.1:9090".into(),
        merchant_pay_to: "11111111111111111111111111111112".into(),
        x402_amount: "1000".into(),
        x402_network: "solana:localnet".into(),
        request_timeout_ms: 5000,
        max_request_bytes: 4096,
        bind_address: "127.0.0.1:8080".into(),
        catalog_path: "config/example.catalog.json".into(),
    }
}

fn test_catalog() -> ServiceCatalog {
    ServiceCatalog::from_entries(vec![ServiceEntry {
        service_id: "hash-demo".into(),
        provider: "11111111111111111111111111111113".into(),
        name: "Hash Demo".into(),
        description: "demo".into(),
        request_schema: "demo.v1".into(),
        x402_demo_route: Some("/v1/x402/services/hash-demo/invoke".into()),
    }])
    .expect("catalog")
}

async fn body_json(body: Body) -> Value {
    let bytes = body.collect().await.expect("body").to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

async fn body_text(body: Body) -> String {
    let bytes = body.collect().await.expect("body").to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn app(reader: Arc<MockChainReader>, fac: Arc<MockFacilitatorClient>) -> axum::Router {
    let state = test_state(
        test_config(),
        test_catalog(),
        reader as Arc<dyn ChainReader>,
        fac as Arc<dyn agentbond_payments::FacilitatorClient>,
    );
    router(state, 4096, Duration::from_secs(5))
}

#[tokio::test]
async fn health_live_and_ready_dependency_failures() {
    let reader = Arc::new(MockChainReader::new());
    let fac = Arc::new(MockFacilitatorClient::new());
    let app = app(reader.clone(), fac.clone());

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("live");
    assert_eq!(res.status(), StatusCode::OK);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("ready");
    assert_eq!(res.status(), StatusCode::OK);

    reader.set_ready(false).await;
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("ready fail");
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = body_json(res.into_body()).await;
    assert!(body["error"]["message"].as_str().is_some());
    assert_eq!(body["error"]["details"]["rpc"], false);

    reader.set_ready(true).await;
    fac.set_ready(false).await;
    let res = app
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("ready fac fail");
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn services_routes_and_invalid_json() {
    let reader = Arc::new(MockChainReader::new());
    let fac = Arc::new(MockFacilitatorClient::new());
    let app = app(reader, fac);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/services")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("services");
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res.into_body()).await;
    assert_eq!(body["services"][0]["service_id"], "hash-demo");

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/services/missing")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("missing");
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let err = body_json(res.into_body()).await;
    assert!(err["error"]["message"].as_str().is_some());

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/plans/jobs/fund")
                .header("content-type", "application/json")
                .body(Body::from("{not-json"))
                .expect("req"),
        )
        .await
        .expect("bad json");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn oversized_body_rejected() {
    let reader = Arc::new(MockChainReader::new());
    let fac = Arc::new(MockFacilitatorClient::new());
    let app = app(reader, fac);
    let big = "x".repeat(8192);
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/plans/jobs/fund")
                .header("content-type", "application/json")
                .body(Body::from(format!(r#"{{"buyer":"{big}","provider":"11111111111111111111111111111112","job_nonce":1}}"#)))
                .expect("req"),
        )
        .await
        .expect("oversized");
    assert!(
        res.status() == StatusCode::PAYLOAD_TOO_LARGE
            || res.status() == StatusCode::BAD_REQUEST
            || res.status() == StatusCode::LENGTH_REQUIRED,
        "unexpected status {}",
        res.status()
    );
}

#[tokio::test]
async fn escrow_plan_routes_never_invoke_facilitator() {
    let reader = Arc::new(MockChainReader::new());
    reader.set_timestamp(1_700_000_000).await;
    let fac = Arc::new(MockFacilitatorClient::new());
    let app = app(reader.clone(), fac.clone());

    let create_body = json!({
        "buyer": "11111111111111111111111111111111",
        "provider": "11111111111111111111111111111112",
        "job_nonce": 1,
        "amount": 1000,
        "request_hash_hex": "09".repeat(32),
        "fund_deadline": 1_700_000_100,
        "accept_deadline": 1_700_000_200,
        "work_deadline": 1_700_000_300,
        "auto_settle_deadline": 1_700_000_400
    });

    for (uri, body) in [
        ("/v1/plans/jobs/create", create_body),
        (
            "/v1/plans/jobs/fund",
            json!({
                "buyer": "11111111111111111111111111111111",
                "provider": "11111111111111111111111111111112",
                "job_nonce": 1
            }),
        ),
        (
            "/v1/plans/jobs/accept",
            json!({
                "buyer": "11111111111111111111111111111111",
                "provider": "11111111111111111111111111111112",
                "job_nonce": 1
            }),
        ),
        (
            "/v1/plans/jobs/accept-work",
            json!({
                "buyer": "11111111111111111111111111111111",
                "provider": "11111111111111111111111111111112",
                "job_nonce": 1
            }),
        ),
        (
            "/v1/plans/jobs/challenge",
            json!({
                "buyer": "11111111111111111111111111111111",
                "provider": "11111111111111111111111111111112",
                "job_nonce": 1,
                "reason_hash_hex": "0a".repeat(32)
            }),
        ),
    ] {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("req"),
            )
            .await
            .expect("plan");
        let status = res.status();
        assert!(
            status.is_success() || status.is_client_error(),
            "{uri} status {status}"
        );
        let text = body_text(res.into_body()).await;
        assert!(!text.to_ascii_lowercase().contains("private_key"), "{uri}");
        // Escrow plan responses must not look like x402 settlement.
        assert!(!text.contains("mock-tx"), "{uri}");
        if status.is_success() {
            let value: Value = serde_json::from_str(&text).expect("json");
            assert!(
                value.get("action").is_some() || value.get("instructions").is_some(),
                "{uri}"
            );
            // x402 route never builds escrow plans — inverse: escrow never builds x402 demo.
            assert_ne!(value.get("action"), Some(&json!("x402_invoke")));
        }
    }

    // Seed job for timeout route.
    let program = program_id();
    let buyer: Pubkey = "11111111111111111111111111111111".parse().expect("buyer");
    let provider: Pubkey = "11111111111111111111111111111112"
        .parse()
        .expect("provider");
    let job_addr = job_pda(&program, &buyer, &provider, 1)
        .expect("job")
        .address;
    let job = JobAccount {
        bump: 1,
        state: JobState::Created,
        buyer: buyer.to_bytes(),
        provider: provider.to_bytes(),
        mint: [0u8; 32],
        token_program: TOKEN_PROGRAM.parse::<Pubkey>().expect("token").to_bytes(),
        amount: 1000,
        job_nonce: 1,
        fund_deadline: 1_699_999_000,
        accept_deadline: 1_700_000_200,
        work_deadline: 1_700_000_300,
        auto_settle_deadline: 1_700_000_400,
        receipt_digest: [0u8; 32],
        request_hash: [9u8; 32],
        locked_bond: 0,
        mint_decimals: 6,
    };
    reader
        .set_account(
            job_addr,
            AccountData {
                owner: program,
                data: job.encode().to_vec(),
                lamports: 1,
            },
        )
        .await;

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/plans/jobs/resolve-timeout")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "payer": "11111111111111111111111111111111",
                        "buyer": "11111111111111111111111111111111",
                        "provider": "11111111111111111111111111111112",
                        "job_nonce": 1
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("timeout");
    assert!(res.status().is_success(), "timeout {}", res.status());

    // submit-receipt with private key field rejected
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/plans/jobs/submit-receipt")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "job": "11111111111111111111111111111111",
                        "provider": "11111111111111111111111111111112",
                        "private_key": "should-reject",
                        "receipt": {
                            "program_id_hex": "0a".repeat(32),
                            "genesis_hash_hex": "07".repeat(32),
                            "job_hex": "01".repeat(32),
                            "buyer_hex": "02".repeat(32),
                            "provider_hex": "03".repeat(32),
                            "request_hash_hex": "09".repeat(32),
                            "result_hash_hex": "04".repeat(32),
                            "artifact_hash_hex": "05".repeat(32),
                            "software_hash_hex": "06".repeat(32),
                            "job_nonce": 1,
                            "created_at": 1,
                            "expires_at": 2
                        },
                        "execution_pubkey_hex": "0b".repeat(32),
                        "signature_hex": "0c".repeat(64)
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("submit");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let err = body_json(res.into_body()).await;
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("private key")
    );

    assert_eq!(fac.verify_calls().await, 0, "escrow must not verify");
    assert_eq!(fac.settle_calls().await, 0, "escrow must not settle");
}

#[tokio::test]
async fn provider_job_inspect_and_x402_never_builds_escrow_plans() {
    let reader = Arc::new(MockChainReader::new());
    let fac = Arc::new(MockFacilitatorClient::new());
    let program = program_id();
    let authority: Pubkey = "11111111111111111111111111111113".parse().expect("auth");
    let provider_addr = provider_pda(&program, &authority).expect("pda").address;
    let provider = ProviderAccount {
        bump: 1,
        status: PROVIDER_STATUS_ACTIVE,
        authority: authority.to_bytes(),
        execution_key_count: 0,
        execution_keys: [[0u8; 32]; 4],
    };
    reader
        .set_account(
            provider_addr,
            AccountData {
                owner: program,
                data: provider.encode().expect("enc").to_vec(),
                lamports: 1,
            },
        )
        .await;

    let app = app(reader.clone(), fac.clone());
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/providers/{provider_addr}"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("provider");
    assert_eq!(res.status(), StatusCode::OK);

    // x402 missing payment -> 402, no escrow plan fields
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/x402/services/hash-demo/invoke")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"input":{"n":1}}"#))
                .expect("req"),
        )
        .await
        .expect("x402");
    assert_eq!(res.status(), StatusCode::PAYMENT_REQUIRED);
    let text = body_text(res.into_body()).await;
    assert!(!text.contains("create_job"));
    assert!(!text.contains("fund_job"));
    assert!(!text.contains("required_signers"));
    assert_eq!(fac.verify_calls().await, 0);

    // Successful x402 payment path uses facilitator, still no escrow plan action.
    let payment = json!({
        "x402Version": 2,
        "resource": {
            "url": "/v1/x402/services/hash-demo/invoke",
            "description": "demo",
            "mimeType": "application/json"
        },
        "accepted": {
            "scheme": "exact",
            "network": "solana:localnet",
            "amount": "1000",
            "asset": "11111111111111111111111111111111",
            "payTo": "11111111111111111111111111111112",
            "maxTimeoutSeconds": 60
        },
        "payload": { "transaction": "deadbeef" }
    });
    let header = Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        payment.to_string().as_bytes(),
    );
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/x402/services/hash-demo/invoke")
                .header("content-type", "application/json")
                .header(PAYMENT_SIGNATURE, header)
                .body(Body::from(r#"{"input":{"n":1}}"#))
                .expect("req"),
        )
        .await
        .expect("x402 paid");
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res.into_body()).await;
    assert_eq!(body["service"], "agentbond-x402-demo");
    assert!(body.get("action").is_none());
    assert!(body.get("instructions").is_none());
    assert!(fac.verify_calls().await >= 1);
    assert!(fac.settle_calls().await >= 1);
}
