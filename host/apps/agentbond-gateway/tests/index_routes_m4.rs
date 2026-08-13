use std::sync::Arc;
use std::time::Duration;

use agentbond_app::{AppConfig, ServiceCatalog, ServiceEntry};
use agentbond_db::Db;
use agentbond_db::test_util::{pg_test_lock, reset_public_tables};
use agentbond_gateway::{router, test_state, test_state_with_db};
use agentbond_indexer::{FixtureSource, IndexerEngine, IndexerMetrics};
use agentbond_payments::MockFacilitatorClient;
use agentbond_sdk::{ChainReader, MockChainReader, program_id};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

fn database_url() -> String {
    std::env::var("AGENTBOND_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://agentbond:agentbond_local_only@127.0.0.1:5433/agentbond".into()
    })
}

fn test_config() -> AppConfig {
    AppConfig {
        program_id: program_id().to_string(),
        rpc_url: "http://127.0.0.1:8899".into(),
        genesis_hash: "07".repeat(32),
        settlement_mint: "11111111111111111111111111111111".into(),
        token_program: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".into(),
        facilitator_url: "http://127.0.0.1:9090".into(),
        merchant_pay_to: "11111111111111111111111111111112".into(),
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

async fn body_json(body: Body) -> Value {
    let bytes = body.collect().await.expect("body").to_bytes();
    serde_json::from_slice(&bytes).expect("json")
}

#[tokio::test]
async fn indexed_endpoints_and_db_unavailable() {
    let reader = Arc::new(MockChainReader::new());
    let fac = Arc::new(MockFacilitatorClient::new());

    // mock mode: index routes report db unavailable
    let mock_app = router(
        test_state(
            test_config(),
            test_catalog(),
            reader.clone() as Arc<dyn ChainReader>,
            fac.clone() as Arc<dyn agentbond_payments::FacilitatorClient>,
        ),
        4_096,
        Duration::from_secs(5),
    );
    let resp = mock_app
        .oneshot(
            Request::builder()
                .uri("/v1/index/status")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(resp.headers().get("x-request-id").is_some());
    let v = body_json(resp.into_body()).await;
    assert!(resp_has_request_id(&v));

    let _lock = pg_test_lock().expect("pg lock");
    let db = Arc::new(
        Db::connect(&database_url())
            .await
            .expect("postgres on 5433"),
    );
    db.migrate().await.expect("migrate");
    reset_public_tables(&db).await.expect("reset");
    let metrics = IndexerMetrics::new().expect("metrics");
    let fixture = std::fs::read_to_string("../fixtures/indexer/lifecycle.json")
        .or_else(|_| std::fs::read_to_string("fixtures/indexer/lifecycle.json"))
        .or_else(|_| {
            std::fs::read_to_string(
                "/Users/ayushkumarmishra/workspace/agentbond-protocol/host/fixtures/indexer/lifecycle.json",
            )
        })
        .expect("fixture file");
    IndexerEngine::new(db.clone(), metrics)
        .run_source(&FixtureSource::from_json(&fixture).expect("fixture"))
        .await
        .expect("replay");

    let app = router(
        test_state_with_db(
            test_config(),
            test_catalog(),
            reader as Arc<dyn ChainReader>,
            fac as Arc<dyn agentbond_payments::FacilitatorClient>,
            db,
        ),
        4_096,
        Duration::from_secs(5),
    );

    let status = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/index/status")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("status");
    assert_eq!(status.status(), StatusCode::OK);
    assert!(status.headers().get("x-request-id").is_some());
    let status_body = body_json(status.into_body()).await;
    assert!(status_body.get("as_of_slot").is_some());

    let jobs = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/index/jobs?limit=1")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("jobs");
    assert_eq!(jobs.status(), StatusCode::OK);
    let jobs_body = body_json(jobs.into_body()).await;
    assert!(jobs_body.get("as_of_slot").is_some());
    let items = jobs_body["items"].as_array().expect("items");
    assert!(!items.is_empty());
    let addr = items[0]["address"].as_str().expect("addr").to_string();
    assert!(items[0]["amount"].as_str().is_some());

    let page2 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/index/jobs?limit=1&cursor={addr}"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("page2");
    assert_eq!(page2.status(), StatusCode::OK);

    let bad_cursor = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/index/jobs?cursor=not-a-key")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("bad");
    assert_eq!(bad_cursor.status(), StatusCode::BAD_REQUEST);

    let hist = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/index/jobs/{addr}/history"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("hist");
    assert_eq!(hist.status(), StatusCode::OK);
    let hist_body = body_json(hist.into_body()).await;
    assert!(!hist_body["items"].as_array().expect("h").is_empty());

    let providers = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/index/providers")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("providers");
    assert_eq!(providers.status(), StatusCode::OK);
    let prov_body = body_json(providers.into_body()).await;
    let paddr = prov_body["items"][0]["address"]
        .as_str()
        .expect("p")
        .to_string();

    let activity = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/index/providers/{paddr}/activity"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("activity");
    assert_eq!(activity.status(), StatusCode::OK);

    // Indexed data must not replace live plan validation: missing on-chain job still fails.
    let plan = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/plans/jobs/submit-receipt")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "job": addr,
                        "receipt": {
                            "program_id": program_id().to_string(),
                            "job": addr,
                            "provider": "11111111111111111111111111111112",
                            "buyer": "11111111111111111111111111111111",
                            "job_nonce": 1,
                            "amount": 1,
                            "work_hash": "11".repeat(32),
                            "output_hash": "22".repeat(32),
                            "issued_at": 1,
                            "expires_at": 2,
                            "signature": "33".repeat(64)
                        }
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("plan");
    assert_ne!(plan.status(), StatusCode::OK);
}

fn resp_has_request_id(v: &Value) -> bool {
    v.pointer("/error/request_id")
        .and_then(|x| x.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}
