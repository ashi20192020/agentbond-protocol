//! Milestone 3 gateway route tests (offline).

use std::sync::Arc;
use std::time::Duration;

use agentbond_app::{AppConfig, ServiceCatalog, ServiceEntry};
use agentbond_gateway::{router, test_state};
use agentbond_payments::headers::{PAYMENT_REQUIRED, PAYMENT_SIGNATURE};
use agentbond_payments::{
    ExactPayloadBody, MockFacilitatorClient, PaymentPayload, PaymentRequired, ResourceInfo,
    SCHEME_EXACT, SvmExactExtra, X402_VERSION,
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

fn test_config() -> AppConfig {
    AppConfig {
        program_id: program_id().to_string(),
        rpc_url: "http://127.0.0.1:8899".into(),
        genesis_hash: "07".repeat(32),
        settlement_mint: "11111111111111111111111111111111".into(),
        token_program: TOKEN_PROGRAM.into(),
        facilitator_url: "http://127.0.0.1:9090".into(),
        merchant_pay_to: "11111111111111111111111111111112".into(),
        x402_fee_payer: "11111111111111111111111111111113".into(),
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

fn app(reader: Arc<MockChainReader>, fac: Arc<MockFacilitatorClient>) -> axum::Router {
    let state = test_state(
        test_config(),
        test_catalog(),
        reader as Arc<dyn ChainReader>,
        fac as Arc<dyn agentbond_payments::FacilitatorClient>,
    );
    router(state, 4096, Duration::from_secs(5))
}

async fn body_json(body: Body) -> Value {
    let bytes = body.collect().await.expect("body").to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[tokio::test]
async fn health_and_request_id() {
    let app = app(
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
    let fac = Arc::new(MockFacilitatorClient::new());
    let app = app(reader, fac);
    let res = app
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("ready");
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(res.headers().get("x-request-id").is_some());
}

#[tokio::test]
async fn structured_provider_and_job() {
    let reader = Arc::new(MockChainReader::new());
    let program = program_id();
    let authority: Pubkey = "11111111111111111111111111111113".parse().expect("pk");
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
    let buyer: Pubkey = "11111111111111111111111111111111".parse().expect("b");
    let job_addr = job_pda(&program, &buyer, &authority, 1)
        .expect("job")
        .address;
    let job = JobAccount {
        bump: 1,
        state: JobState::Created,
        buyer: buyer.to_bytes(),
        provider: authority.to_bytes(),
        mint: [0u8; 32],
        token_program: TOKEN_PROGRAM.parse::<Pubkey>().expect("t").to_bytes(),
        amount: 1000,
        job_nonce: 1,
        fund_deadline: 10,
        accept_deadline: 20,
        work_deadline: 30,
        auto_settle_deadline: 40,
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

    let app = app(reader, Arc::new(MockFacilitatorClient::new()));
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
async fn plan_routes_and_private_key_rejection() {
    let reader = Arc::new(MockChainReader::new());
    reader.set_timestamp(1_700_000_000).await;
    let fac = Arc::new(MockFacilitatorClient::new());
    let app = app(reader, fac.clone());

    let create = json!({
        "buyer": "11111111111111111111111111111111",
        "provider": "11111111111111111111111111111112",
        "job_nonce": 1,
        "amount": 1000,
        "request_hash_hex": "09".repeat(32),
        "fund_deadline": 1_700_000_100i64,
        "accept_deadline": 1_700_000_200i64,
        "work_deadline": 1_700_000_300i64,
        "auto_settle_deadline": 1_700_000_400i64
    });
    for path in [
        "/v1/plans/jobs/create",
        "/v1/plans/jobs/fund",
        "/v1/plans/jobs/accept",
        "/v1/plans/jobs/accept-work",
        "/v1/plans/jobs/challenge",
        "/v1/plans/jobs/resolve-timeout",
        "/v1/plans/jobs/submit-receipt",
    ] {
        let body = if path.ends_with("create") {
            create.clone()
        } else if path.ends_with("challenge") {
            json!({
                "buyer":"11111111111111111111111111111111",
                "provider":"11111111111111111111111111111112",
                "job_nonce":1,
                "reason_hash_hex":"0a".repeat(32)
            })
        } else if path.ends_with("resolve-timeout") {
            json!({
                "payer":"11111111111111111111111111111111",
                "buyer":"11111111111111111111111111111111",
                "provider":"11111111111111111111111111111112",
                "job_nonce":1
            })
        } else if path.ends_with("submit-receipt") {
            json!({
                "job":"11111111111111111111111111111114",
                "provider":"11111111111111111111111111111112",
                "receipt":{
                    "program_id_hex": hex::encode(program_id().to_bytes()),
                    "genesis_hash_hex":"07".repeat(32),
                    "job_hex":"0e".repeat(32),
                    "buyer_hex":"02".repeat(32),
                    "provider_hex": hex::encode("11111111111111111111111111111112".parse::<Pubkey>().expect("p").to_bytes()),
                    "request_hash_hex":"09".repeat(32),
                    "result_hash_hex":"04".repeat(32),
                    "artifact_hash_hex":"05".repeat(32),
                    "software_hash_hex":"06".repeat(32),
                    "job_nonce":1,
                    "created_at":1_700_000_000i64,
                    "expires_at":1_700_000_400i64
                },
                "execution_pubkey_hex":"0b".repeat(32),
                "signature_hex":"0c".repeat(64)
            })
        } else {
            json!({
                "buyer":"11111111111111111111111111111111",
                "provider":"11111111111111111111111111111112",
                "job_nonce":1
            })
        };
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("req"),
            )
            .await
            .expect("plan");
        // create may succeed; others may fail validation — never 500, never call facilitator
        assert_ne!(res.status(), StatusCode::INTERNAL_SERVER_ERROR, "{path}");
        assert!(res.headers().get("x-request-id").is_some());
    }
    assert_eq!(fac.verify_calls().await, 0);
    assert_eq!(fac.settle_calls().await, 0);

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/plans/jobs/fund")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"buyer":"11111111111111111111111111111111","provider":"11111111111111111111111111111112","job_nonce":1,"private_key":"aa"}).to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("reject");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn x402_missing_and_success() {
    let reader = Arc::new(MockChainReader::new());
    reader.set_timestamp(1_700_000_000).await;
    let fac = Arc::new(MockFacilitatorClient::new());
    let app = app(reader, fac.clone());

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/x402/services/hash-demo/invoke")
                .header("content-type", "application/json")
                .body(Body::from(json!({"input":{"x":1}}).to_string()))
                .expect("req"),
        )
        .await
        .expect("402");
    assert_eq!(res.status(), StatusCode::PAYMENT_REQUIRED);
    let payment_required = res
        .headers()
        .get(PAYMENT_REQUIRED)
        .expect("PAYMENT-REQUIRED")
        .to_str()
        .expect("str")
        .to_string();
    assert_eq!(fac.settle_calls().await, 0);

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
            transaction: Engine::encode(&base64::engine::general_purpose::STANDARD, [9u8; 64]),
        },
        extensions: Default::default(),
    };
    let sig = Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        serde_json::to_vec(&payload).expect("json"),
    );
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/x402/services/hash-demo/invoke")
                .header("content-type", "application/json")
                .header(PAYMENT_SIGNATURE, sig)
                .body(Body::from(json!({"input":{"x":1}}).to_string()))
                .expect("req"),
        )
        .await
        .expect("200");
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(fac.settle_calls().await, 1);
}

mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
    }
}
