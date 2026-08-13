use std::sync::Arc;
use std::time::Duration;

use agentbond_app::{AppConfig, ServiceCatalog, ServiceEntry};
use agentbond_gateway::{router, test_state};
use agentbond_payments::{
    ExactPayloadBody, MockFacilitatorClient, PAYMENT_REQUIRED, PAYMENT_SIGNATURE, PaymentPayload,
    PaymentRequired, ResourceInfo, SCHEME_EXACT, SvmExactExtra, X402_VERSION,
};
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
const BUYER: &str = "11111111111111111111111111111111";
const PROVIDER: &str = "11111111111111111111111111111112";

fn test_config() -> AppConfig {
    AppConfig {
        program_id: program_id().to_string(),
        rpc_url: "http://127.0.0.1:8899".into(),
        genesis_hash: "07".repeat(32),
        settlement_mint: BUYER.into(),
        token_program: TOKEN_PROGRAM.into(),
        facilitator_url: "http://127.0.0.1:9090".into(),
        merchant_pay_to: PROVIDER.into(),
        x402_fee_payer: "11111111111111111111111111111113".into(),
        x402_amount: "1000".into(),
        x402_network: "solana:localnet".into(),
        request_timeout_ms: 5_000,
        max_request_bytes: 4_096,
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
        request_schema: "demo".into(),
        x402_demo_route: Some("/v1/x402/services/hash-demo/invoke".into()),
    }])
    .expect("catalog")
}

fn make_app(
    reader: Arc<MockChainReader>,
    fac: Arc<MockFacilitatorClient>,
    max_body: usize,
    timeout: Duration,
) -> axum::Router {
    let state = test_state(
        test_config(),
        test_catalog(),
        reader as Arc<dyn ChainReader>,
        fac as Arc<dyn agentbond_payments::FacilitatorClient>,
    );
    router(state, max_body, timeout)
}

fn default_app(reader: Arc<MockChainReader>, fac: Arc<MockFacilitatorClient>) -> axum::Router {
    make_app(reader, fac, 4_096, Duration::from_secs(5))
}

async fn body_json(body: Body) -> Value {
    let bytes = body.collect().await.expect("body").to_bytes();
    serde_json::from_slice(&bytes).expect("json")
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn buyer() -> Pubkey {
    BUYER.parse().expect("buyer")
}
fn provider() -> Pubkey {
    PROVIDER.parse().expect("provider")
}
fn token_program() -> Pubkey {
    TOKEN_PROGRAM.parse().expect("token")
}

fn assert_structured_error(status: StatusCode, headers: &axum::http::HeaderMap, body: &Value) {
    assert!(headers.get("x-request-id").is_some());
    assert!(body["error"]["code"].as_str().expect("code").len() > 1);
    assert!(body["error"]["message"].as_str().expect("msg").len() > 1);
    let rid = body["error"]["request_id"].as_str().expect("rid");
    assert_ne!(rid, "unknown");
    assert_eq!(
        headers.get("x-request-id").and_then(|v| v.to_str().ok()),
        Some(rid)
    );
    assert!(!format!("{body}").to_ascii_lowercase().contains("stack"));
    let _ = status;
}

async fn seed_provider(reader: &MockChainReader) -> Pubkey {
    let program = program_id();
    let authority = provider();
    let addr = provider_pda(&program, &authority).expect("pda").address;
    let account = ProviderAccount {
        bump: 1,
        status: PROVIDER_STATUS_ACTIVE,
        authority: authority.to_bytes(),
        execution_key_count: 0,
        execution_keys: [[0u8; 32]; 4],
    };
    reader
        .set_account(
            addr,
            AccountData {
                owner: program,
                data: account.encode().expect("enc").to_vec(),
                lamports: 1,
            },
        )
        .await;
    addr
}

async fn seed_job(reader: &MockChainReader, state: JobState) -> Pubkey {
    let program = program_id();
    let buyer = buyer();
    let provider = provider();
    let addr = job_pda(&program, &buyer, &provider, 1)
        .expect("job")
        .address;
    let job = JobAccount {
        bump: 1,
        state,
        buyer: buyer.to_bytes(),
        provider: provider.to_bytes(),
        mint: buyer.to_bytes(),
        token_program: token_program().to_bytes(),
        amount: 1000,
        job_nonce: 1,
        fund_deadline: 1_700_000_100,
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
            addr,
            AccountData {
                owner: program,
                data: job.encode().to_vec(),
                lamports: 1,
            },
        )
        .await;
    addr
}

async fn post_json(app: &axum::Router, path: &str, body: Value) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("req"),
        )
        .await
        .expect("response")
}

async fn issue_payment_sig(app: &axum::Router, input: Value, tx_byte: u8) -> String {
    let res = post_json(
        app,
        "/v1/x402/services/hash-demo/invoke",
        json!({"input": input}),
    )
    .await;
    assert_eq!(res.status(), StatusCode::PAYMENT_REQUIRED);
    let payment_required = res
        .headers()
        .get(PAYMENT_REQUIRED)
        .expect("PAYMENT-REQUIRED")
        .to_str()
        .expect("str")
        .to_string();
    let required: PaymentRequired = {
        let bytes = Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            payment_required.trim(),
        )
        .expect("b64");
        serde_json::from_slice(&bytes).expect("json")
    };
    let accepted = required.accepts[0].clone();
    let payload = PaymentPayload {
        x402_version: X402_VERSION,
        resource: ResourceInfo {
            url: "/v1/x402/services/hash-demo/invoke".into(),
            description: "demo".into(),
            mime_type: "application/json".into(),
        },
        accepted: agentbond_payments::PaymentRequirements {
            scheme: SCHEME_EXACT.into(),
            network: accepted.network,
            amount: accepted.amount,
            asset: accepted.asset,
            pay_to: accepted.pay_to,
            max_timeout_seconds: accepted.max_timeout_seconds,
            extra: SvmExactExtra {
                fee_payer: accepted.extra.fee_payer,
                memo: accepted.extra.memo,
                recent_blockhash: None,
                last_valid_block_height: None,
            },
        },
        payload: ExactPayloadBody {
            transaction: Engine::encode(&base64::engine::general_purpose::STANDARD, [tx_byte; 64]),
        },
        extensions: Default::default(),
    };
    Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        serde_json::to_vec(&payload).expect("json"),
    )
}

async fn paid_invoke(app: &axum::Router, sig: &str, input: Value) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/x402/services/hash-demo/invoke")
                .header("content-type", "application/json")
                .header(PAYMENT_SIGNATURE, sig)
                .body(Body::from(json!({"input": input}).to_string()))
                .expect("req"),
        )
        .await
        .expect("resp")
}

#[tokio::test]
async fn health_and_request_id() {
    let app = default_app(
        Arc::new(MockChainReader::new()),
        Arc::new(MockFacilitatorClient::new()),
    );
    let res = app
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("live");
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers().get("x-request-id").is_some());
}

#[tokio::test]
async fn readiness_dependency_failure() {
    let reader = Arc::new(MockChainReader::new());
    reader.set_ready(false).await;
    let app = default_app(reader, Arc::new(MockFacilitatorClient::new()));
    let res = app
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("ready");
    let status = res.status();
    let headers = res.headers().clone();
    let body = body_json(res.into_body()).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_structured_error(status, &headers, &body);
}

#[tokio::test]
async fn structured_provider_and_job() {
    let reader = Arc::new(MockChainReader::new());
    let provider_addr = seed_provider(&reader).await;
    let job_addr = seed_job(&reader, JobState::Created).await;
    let app = default_app(reader, Arc::new(MockFacilitatorClient::new()));

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
    let v = body_json(res.into_body()).await;
    assert_eq!(v["provider"]["status"], "Active");

    let res = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/jobs/{job_addr}"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("job");
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res.into_body()).await;
    assert_eq!(v["job"]["state"], "Created");
}

#[tokio::test]
async fn every_plan_route_success() {
    let reader = Arc::new(MockChainReader::new());
    reader.set_timestamp(1_700_000_000).await;
    let _ = seed_provider(&reader).await;
    let job_addr = seed_job(&reader, JobState::Created).await;
    let fac = Arc::new(MockFacilitatorClient::new());
    let app = default_app(reader.clone(), fac.clone());

    let cases = vec![
        (
            "/v1/plans/jobs/create",
            json!({
                "buyer": BUYER,
                "provider": PROVIDER,
                "job_nonce": 1,
                "amount": 1000,
                "request_hash_hex": "09".repeat(32),
                "fund_deadline": 1_700_000_100i64,
                "accept_deadline": 1_700_000_200i64,
                "work_deadline": 1_700_000_300i64,
                "auto_settle_deadline": 1_700_000_400i64
            }),
            "create_job",
            1usize,
        ),
        (
            "/v1/plans/jobs/fund",
            json!({"buyer": BUYER, "provider": PROVIDER, "job_nonce": 1}),
            "fund_job",
            1,
        ),
        (
            "/v1/plans/jobs/accept",
            json!({"buyer": BUYER, "provider": PROVIDER, "job_nonce": 1}),
            "accept_job",
            1,
        ),
        (
            "/v1/plans/jobs/accept-work",
            json!({"buyer": BUYER, "provider": PROVIDER, "job_nonce": 1}),
            "accept_work",
            1,
        ),
        (
            "/v1/plans/jobs/challenge",
            json!({
                "buyer": BUYER,
                "provider": PROVIDER,
                "job_nonce": 1,
                "reason_hash_hex": "0a".repeat(32)
            }),
            "challenge_work",
            1,
        ),
        (
            "/v1/plans/jobs/submit-receipt",
            json!({
                "job": job_addr.to_string(),
                "provider": PROVIDER,
                "receipt": {
                    "program_id_hex": hex_encode(&program_id().to_bytes()),
                    "genesis_hash_hex": "07".repeat(32),
                    "job_hex": hex_encode(&job_addr.to_bytes()),
                    "buyer_hex": hex_encode(&buyer().to_bytes()),
                    "provider_hex": hex_encode(&provider().to_bytes()),
                    "request_hash_hex": "09".repeat(32),
                    "result_hash_hex": "04".repeat(32),
                    "artifact_hash_hex": "05".repeat(32),
                    "software_hash_hex": "06".repeat(32),
                    "job_nonce": 1,
                    "created_at": 1_700_000_000i64,
                    "expires_at": 1_700_000_400i64
                },
                "execution_pubkey_hex": "0b".repeat(32),
                "signature_hex": "0c".repeat(64)
            }),
            "submit_receipt",
            2,
        ),
    ];

    for (path, body, action, min_ix) in cases {
        let res = post_json(&app, path, body).await;
        let status = res.status();
        let headers = res.headers().clone();
        let v = body_json(res.into_body()).await;
        assert_eq!(status, StatusCode::OK, "{path} {v}");
        assert!(headers.get("x-request-id").is_some());
        assert_eq!(v["action"], action);
        assert_eq!(v["program_id"], program_id().to_string());
        assert!(v["instructions"].as_array().expect("ix").len() >= min_ix);
        let signers = v["required_signers"].as_array().expect("signers");
        if action != "submit_receipt" {
            assert!(!signers.is_empty(), "{path} missing required signers");
        }
        assert!(v.get("private_key").is_none());
    }

    reader.set_timestamp(1_700_000_100).await;
    let res = post_json(
        &app,
        "/v1/plans/jobs/resolve-timeout",
        json!({
            "payer": BUYER,
            "buyer": BUYER,
            "provider": PROVIDER,
            "job_nonce": 1
        }),
    )
    .await;
    let status = res.status();
    let headers = res.headers().clone();
    let v = body_json(res.into_body()).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert!(headers.get("x-request-id").is_some());
    assert_eq!(v["action"], "expire_unfunded");
    assert_eq!(v["program_id"], program_id().to_string());
    assert!(!v["instructions"].as_array().expect("ix").is_empty());
    assert!(!v["required_signers"].as_array().expect("s").is_empty());
    assert_eq!(fac.verify_calls().await, 0);
    assert_eq!(fac.settle_calls().await, 0);
}

#[tokio::test]
async fn plan_boundary_failures() {
    let reader = Arc::new(MockChainReader::new());
    reader.set_timestamp(1_700_000_000).await;
    let _job_addr = seed_job(&reader, JobState::Created).await;
    let fac = Arc::new(MockFacilitatorClient::new());
    let app = default_app(reader.clone(), fac.clone());

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/plans/jobs/fund")
                .header("content-type", "application/json")
                .body(Body::from("{not-json"))
                .expect("req"),
        )
        .await
        .expect("malformed");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert!(res.headers().get("x-request-id").is_some());

    let res = post_json(
        &app,
        "/v1/plans/jobs/fund",
        json!({"buyer":"not-a-key","provider": PROVIDER, "job_nonce": 1}),
    )
    .await;
    let status = res.status();
    let headers = res.headers().clone();
    let body = body_json(res.into_body()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_structured_error(status, &headers, &body);

    let res = post_json(
        &app,
        "/v1/plans/jobs/fund",
        json!({"buyer": BUYER, "provider": PROVIDER, "job_nonce": -1}),
    )
    .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let res = post_json(
        &app,
        "/v1/plans/jobs/fund",
        json!({"buyer": BUYER, "provider": PROVIDER, "job_nonce": 1, "private_key": "aa"}),
    )
    .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let missing_provider = "11111111111111111111111111111114";
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/providers/{missing_provider}"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("missing provider");
    let status = res.status();
    let headers = res.headers().clone();
    let body = body_json(res.into_body()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_structured_error(status, &headers, &body);

    let missing_job = "11111111111111111111111111111115";
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/jobs/{missing_job}"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("missing job");
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let wrong_provider = "11111111111111111111111111111113";
    let res = post_json(
        &app,
        "/v1/plans/jobs/resolve-timeout",
        json!({
            "payer": BUYER,
            "buyer": BUYER,
            "provider": wrong_provider,
            "job_nonce": 1
        }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = post_json(
        &app,
        "/v1/plans/jobs/resolve-timeout",
        json!({
            "payer": BUYER,
            "buyer": BUYER,
            "provider": PROVIDER,
            "job_nonce": 1
        }),
    )
    .await;
    let status = res.status();
    let headers = res.headers().clone();
    let body = body_json(res.into_body()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_structured_error(status, &headers, &body);

    reader.set_fail_rpc(true).await;
    let res = post_json(
        &app,
        "/v1/plans/jobs/create",
        json!({
            "buyer": BUYER,
            "provider": PROVIDER,
            "job_nonce": 1,
            "amount": 1000,
            "request_hash_hex": "09".repeat(32),
            "fund_deadline": 1_700_000_100i64,
            "accept_deadline": 1_700_000_200i64,
            "work_deadline": 1_700_000_300i64,
            "auto_settle_deadline": 1_700_000_400i64
        }),
    )
    .await;
    let status = res.status();
    let headers = res.headers().clone();
    let body = body_json(res.into_body()).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_structured_error(status, &headers, &body);
    reader.set_fail_rpc(false).await;

    let huge = "x".repeat(5_000);
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/plans/jobs/fund")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"buyer":"{BUYER}","provider":"{PROVIDER}","job_nonce":1,"pad":"{huge}"}}"#
                )))
                .expect("req"),
        )
        .await
        .expect("limit");
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let slow = Arc::new(MockChainReader::new());
    slow.set_delay(Some(Duration::from_millis(200))).await;
    let timed = make_app(slow, fac.clone(), 4_096, Duration::from_millis(50));
    let res = post_json(
        &timed,
        "/v1/plans/jobs/create",
        json!({
            "buyer": BUYER,
            "provider": PROVIDER,
            "job_nonce": 1,
            "amount": 1000,
            "request_hash_hex": "09".repeat(32),
            "fund_deadline": 1_700_000_100i64,
            "accept_deadline": 1_700_000_200i64,
            "work_deadline": 1_700_000_300i64,
            "auto_settle_deadline": 1_700_000_400i64
        }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::REQUEST_TIMEOUT);
    assert_eq!(fac.settle_calls().await, 0);
}

#[tokio::test]
async fn wrong_job_provider_binding_on_submit() {
    let reader = Arc::new(MockChainReader::new());
    reader.set_timestamp(1_700_000_000).await;
    let job_addr = seed_job(&reader, JobState::Accepted).await;
    let app = default_app(reader, Arc::new(MockFacilitatorClient::new()));
    let res = post_json(
        &app,
        "/v1/plans/jobs/submit-receipt",
        json!({
            "job": job_addr.to_string(),
            "provider": "11111111111111111111111111111113",
            "receipt": {
                "program_id_hex": hex_encode(&program_id().to_bytes()),
                "genesis_hash_hex": "07".repeat(32),
                "job_hex": hex_encode(&job_addr.to_bytes()),
                "buyer_hex": hex_encode(&buyer().to_bytes()),
                "provider_hex": hex_encode(&provider().to_bytes()),
                "request_hash_hex": "09".repeat(32),
                "result_hash_hex": "04".repeat(32),
                "artifact_hash_hex": "05".repeat(32),
                "software_hash_hex": "06".repeat(32),
                "job_nonce": 1,
                "created_at": 1_700_000_000i64,
                "expires_at": 1_700_000_400i64
            },
            "execution_pubkey_hex": "0b".repeat(32),
            "signature_hex": "0c".repeat(64)
        }),
    )
    .await;
    let status = res.status();
    let headers = res.headers().clone();
    let body = body_json(res.into_body()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_structured_error(status, &headers, &body);
}

#[tokio::test]
async fn x402_http_matrix() {
    let reader = Arc::new(MockChainReader::new());
    reader.set_timestamp(1_700_000_000).await;
    let fac = Arc::new(MockFacilitatorClient::new());
    let app = default_app(reader, fac.clone());

    let sig = issue_payment_sig(&app, json!({"x": 1}), 9).await;
    assert_eq!(fac.verify_calls().await, 0);

    let res = paid_invoke(&app, &sig, json!({"x": 1})).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(fac.settle_calls().await, 1);
    let first_body = body_json(res.into_body()).await;

    let res = paid_invoke(&app, &sig, json!({"x": 1})).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(fac.settle_calls().await, 1);
    assert_eq!(body_json(res.into_body()).await, first_body);

    let res = paid_invoke(&app, &sig, json!({"x": 2})).await;
    let status = res.status();
    let headers = res.headers().clone();
    let body = body_json(res.into_body()).await;
    assert_ne!(status, StatusCode::OK);
    assert_structured_error(status, &headers, &body);

    let concurrent_sig = Arc::new(issue_payment_sig(&app, json!({"c": 1}), 8).await);
    let before = fac.settle_calls().await;
    let mut joins = Vec::new();
    for _ in 0..8 {
        let app = app.clone();
        let sig = concurrent_sig.clone();
        joins.push(tokio::spawn(async move {
            paid_invoke(&app, sig.as_str(), json!({"c": 1})).await
        }));
    }
    let mut oks = 0u32;
    for j in joins {
        if j.await.expect("join").status() == StatusCode::OK {
            oks += 1;
        }
    }
    assert!(oks >= 1);
    assert_eq!(fac.settle_calls().await, before + 1);

    fac.set_verify_ok(false).await;
    let bad_verify = issue_payment_sig(&app, json!({"v": 1}), 7).await;
    let res = paid_invoke(&app, &bad_verify, json!({"v": 1})).await;
    let status = res.status();
    let headers = res.headers().clone();
    let body = body_json(res.into_body()).await;
    assert_ne!(status, StatusCode::OK);
    assert_structured_error(status, &headers, &body);
    fac.set_verify_ok(true).await;

    fac.set_settle_ok(false).await;
    let bad_settle = issue_payment_sig(&app, json!({"s": 1}), 6).await;
    let res = paid_invoke(&app, &bad_settle, json!({"s": 1})).await;
    let status = res.status();
    let headers = res.headers().clone();
    let body = body_json(res.into_body()).await;
    assert_ne!(status, StatusCode::OK);
    assert_structured_error(status, &headers, &body);
    fac.set_settle_ok(true).await;

    fac.set_verify_delay(Some(Duration::from_millis(1))).await;
    let timeout_sig = issue_payment_sig(&app, json!({"t": 1}), 5).await;
    let res = paid_invoke(&app, &timeout_sig, json!({"t": 1})).await;
    let status = res.status();
    let headers = res.headers().clone();
    let body = body_json(res.into_body()).await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(body["error"]["code"], "verify_timeout");
    assert_structured_error(status, &headers, &body);
    fac.set_verify_delay(None).await;

    let verify_before = fac.verify_calls().await;
    let settle_before = fac.settle_calls().await;
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/x402/services/hash-demo/invoke")
                .header("content-type", "application/json")
                .header(PAYMENT_SIGNATURE, "%%%")
                .body(Body::from(json!({"input":{"bad":1}}).to_string()))
                .expect("req"),
        )
        .await
        .expect("rejected");
    assert_ne!(res.status(), StatusCode::OK);
    assert_eq!(fac.verify_calls().await, verify_before);
    assert_eq!(fac.settle_calls().await, settle_before);
}
