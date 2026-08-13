use std::sync::Arc;
use std::time::Duration;

use agentbond_app::{AppConfig, ServiceCatalog, ServiceEntry};
use agentbond_db::Db;
use agentbond_db::test_util::{pg_test_lock, reset_public_tables};
use agentbond_gateway::{router, test_state, test_state_with_db};
use agentbond_indexer::{FixtureSource, IndexerEngine, IndexerMetrics};
use agentbond_payments::{
    ExactPayloadBody, MockFacilitatorClient, PAYMENT_REQUIRED, PAYMENT_SIGNATURE, PaymentPayload,
    PaymentRequired, ResourceInfo, SCHEME_EXACT, SvmExactExtra, X402_VERSION,
};
use agentbond_sdk::{ChainReader, MockChainReader, program_id};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine;
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

async fn body_text(body: Body) -> String {
    let bytes = body.collect().await.expect("body").to_bytes();
    String::from_utf8(bytes.to_vec()).expect("utf8")
}

fn resp_has_request_id(v: &Value) -> bool {
    v.pointer("/error/request_id")
        .and_then(|x| x.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

async fn seeded_app() -> (
    std::fs::File,
    axum::Router,
    Arc<Db>,
    Arc<MockChainReader>,
    Arc<MockFacilitatorClient>,
) {
    let lock = pg_test_lock().expect("pg lock");
    let db = Arc::new(
        Db::connect(&database_url())
            .await
            .expect("postgres on 5433"),
    );
    db.migrate().await.expect("migrate");
    reset_public_tables(&db).await.expect("reset");
    let metrics = IndexerMetrics::new().expect("metrics");
    let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/indexer/lifecycle.json");
    let fixture = std::fs::read_to_string(&fixture_path).expect("fixture file");
    IndexerEngine::new(db.clone(), metrics)
        .run_source(&FixtureSource::from_json(&fixture).expect("fixture"))
        .await
        .expect("replay");

    let reader = Arc::new(MockChainReader::new());
    reader.set_timestamp(1_700_000_000).await;
    let fac = Arc::new(MockFacilitatorClient::new());
    let app = router(
        test_state_with_db(
            test_config(),
            test_catalog(),
            reader.clone() as Arc<dyn ChainReader>,
            fac.clone() as Arc<dyn agentbond_payments::FacilitatorClient>,
            db.clone(),
        ),
        4_096,
        Duration::from_secs(5),
    );
    (lock, app, db, reader, fac)
}

async fn issue_payment_sig(app: &axum::Router, input: Value, tx_byte: u8) -> String {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/x402/services/hash-demo/invoke")
                .header("content-type", "application/json")
                .body(Body::from(json!({"input": input}).to_string()))
                .expect("req"),
        )
        .await
        .expect("resp");
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
        serde_json::to_vec(&payload).expect("payload"),
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
async fn indexed_status_and_db_unavailable() {
    let reader = Arc::new(MockChainReader::new());
    let fac = Arc::new(MockFacilitatorClient::new());
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

    let (_lock, app, _db, _reader, _fac) = seeded_app().await;
    let status = app
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
}

#[tokio::test]
async fn indexed_jobs_validation_and_pagination() {
    let (_lock, app, _db, _reader, _fac) = seeded_app().await;

    for uri in ["/v1/index/jobs?limit=0", "/v1/index/jobs?limit=101"] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("req"),
            )
            .await
            .expect("resp");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{uri}");
        let v = body_json(resp.into_body()).await;
        assert!(resp_has_request_id(&v));
    }

    let bad_state = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/index/jobs?state=Pending")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("bad state");
    assert_eq!(bad_state.status(), StatusCode::BAD_REQUEST);

    let bad_buyer = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/index/jobs?buyer=not-a-pubkey")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("bad buyer");
    assert_eq!(bad_buyer.status(), StatusCode::BAD_REQUEST);

    let bad_provider = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/index/jobs?provider=zzzz")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("bad provider");
    assert_eq!(bad_provider.status(), StatusCode::BAD_REQUEST);

    let bad_cursor = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/index/jobs?cursor=not-a-key")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("bad cursor");
    assert_eq!(bad_cursor.status(), StatusCode::BAD_REQUEST);

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
    let as_of = jobs_body["as_of_slot"].as_str().expect("as_of");
    for item in items {
        let row_slot = item["as_of_slot"].as_str().expect("row as_of");
        assert!(
            row_slot.parse::<u64>().unwrap() <= as_of.parse::<u64>().unwrap(),
            "row newer than reported as_of_slot"
        );
    }
    let addr = items[0]["address"].as_str().expect("addr").to_string();
    assert!(jobs_body.get("next_cursor").is_some());

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
    let page2_body = body_json(page2.into_body()).await;
    assert_eq!(page2_body["as_of_slot"], jobs_body["as_of_slot"]);

    let all = app
        .oneshot(
            Request::builder()
                .uri("/v1/index/jobs?limit=100")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("all");
    assert_eq!(all.status(), StatusCode::OK);
    let all_body = body_json(all.into_body()).await;
    if all_body["items"].as_array().expect("items").len() < 100 {
        assert!(
            all_body.get("next_cursor").unwrap().is_null() || all_body["next_cursor"].is_null()
        );
    }
}

#[tokio::test]
async fn indexed_job_history_route() {
    let (_lock, app, _db, _reader, _fac) = seeded_app().await;
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
    let jobs_body = body_json(jobs.into_body()).await;
    let addr = jobs_body["items"][0]["address"]
        .as_str()
        .expect("addr")
        .to_string();
    let as_of = jobs_body["as_of_slot"].clone();

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
    assert_eq!(hist_body["as_of_slot"], as_of);
    assert!(!hist_body["items"].as_array().expect("h").is_empty());

    let bad_event_cursor = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/index/jobs/{addr}/history?cursor=bad"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("bad event cursor");
    assert_eq!(bad_event_cursor.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn indexed_providers_and_activity_routes() {
    let (_lock, app, _db, _reader, _fac) = seeded_app().await;

    let providers = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/index/providers?limit=0")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("bad limit");
    assert_eq!(providers.status(), StatusCode::BAD_REQUEST);

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
    assert!(prov_body.get("as_of_slot").is_some());
    let paddr = prov_body["items"][0]["address"]
        .as_str()
        .expect("p")
        .to_string();

    let activity = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/index/providers/{paddr}/activity"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("activity");
    assert_eq!(activity.status(), StatusCode::OK);
    let act_body = body_json(activity.into_body()).await;
    assert_eq!(act_body["as_of_slot"], prov_body["as_of_slot"]);
}

#[tokio::test]
async fn indexed_data_does_not_replace_live_plan_validation() {
    let (_lock, app, _db, _reader, _fac) = seeded_app().await;
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
    let jobs_body = body_json(jobs.into_body()).await;
    let addr = jobs_body["items"][0]["address"]
        .as_str()
        .expect("addr")
        .to_string();

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

#[tokio::test]
async fn gateway_metrics_and_ready_endpoints() {
    let (_lock, app, _db, _reader, fac) = seeded_app().await;

    let live = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("live");
    assert_eq!(live.status(), StatusCode::OK);

    let ready = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("ready");
    assert_eq!(ready.status(), StatusCode::OK);

    let metrics_before = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("metrics");
    assert_eq!(metrics_before.status(), StatusCode::OK);
    assert_eq!(
        metrics_before
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("text/plain; version=0.0.4; charset=utf-8")
    );
    let before = body_text(metrics_before.into_body()).await;
    assert!(before.contains("agentbond_settlement_lease_acquisition"));

    let input = json!({"metrics": true});
    let sig = issue_payment_sig(&app, input.clone(), 7).await;
    let paid = paid_invoke(&app, &sig, input).await;
    assert_eq!(paid.status(), StatusCode::OK);
    assert_eq!(fac.settle_calls().await, 1);

    let metrics_after = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("metrics after");
    let after = body_text(metrics_after.into_body()).await;
    assert!(
        after.contains("agentbond_settlement_completion")
            && after.contains("agentbond_settlement_lease_acquisition"),
        "settlement counters must appear after paid invoke"
    );
    let acq = metric_value(&after, "agentbond_settlement_lease_acquisition");
    let done = metric_value(&after, "agentbond_settlement_completion");
    assert!(acq >= 1.0, "lease acquisition should increment");
    assert!(done >= 1.0, "completion should increment");
}

#[tokio::test]
async fn two_gateway_states_share_postgres_settle_once() {
    let _lock = pg_test_lock().expect("pg lock");
    let db = Arc::new(
        Db::connect(&database_url())
            .await
            .expect("postgres on 5433"),
    );
    db.migrate().await.expect("migrate");
    reset_public_tables(&db).await.expect("reset");

    let reader = Arc::new(MockChainReader::new());
    reader.set_timestamp(1_700_000_000).await;
    let fac = Arc::new(MockFacilitatorClient::new());
    let app_a = router(
        test_state_with_db(
            test_config(),
            test_catalog(),
            reader.clone() as Arc<dyn ChainReader>,
            fac.clone() as Arc<dyn agentbond_payments::FacilitatorClient>,
            db.clone(),
        ),
        4_096,
        Duration::from_secs(5),
    );
    let app_b = router(
        test_state_with_db(
            test_config(),
            test_catalog(),
            reader.clone() as Arc<dyn ChainReader>,
            fac.clone() as Arc<dyn agentbond_payments::FacilitatorClient>,
            db.clone(),
        ),
        4_096,
        Duration::from_secs(5),
    );

    let input = json!({"shared": true});
    let sig = issue_payment_sig(&app_a, input.clone(), 9).await;

    let entered = fac.arm_settle_hold().await;
    let app_a2 = app_a.clone();
    let app_b2 = app_b.clone();
    let sig_a = sig.clone();
    let sig_b = sig.clone();
    let input_a = input.clone();
    let input_b = input.clone();
    let t_a = tokio::spawn(async move { paid_invoke(&app_a2, &sig_a, input_a).await });
    let t_b = tokio::spawn(async move { paid_invoke(&app_b2, &sig_b, input_b).await });

    // Wait until the winner reaches facilitator settle, then collect the loser.
    entered.notified().await;
    let either = futures::future::select(t_a, t_b).await;
    let (loser, winner_handle) = match either {
        futures::future::Either::Left((r, other)) => (r.expect("join"), other),
        futures::future::Either::Right((r, other)) => (r.expect("join"), other),
    };
    assert_eq!(
        loser.status(),
        StatusCode::CONFLICT,
        "overlapping request must report in-progress"
    );
    assert_eq!(fac.settle_calls().await, 1);

    fac.release_settle_hold().await;
    let winner = winner_handle.await.expect("join winner");
    assert_eq!(winner.status(), StatusCode::OK);
    assert_eq!(fac.settle_calls().await, 1);

    let cached = paid_invoke(&app_b, &sig, input).await;
    assert_eq!(cached.status(), StatusCode::OK);
    assert_eq!(fac.settle_calls().await, 1);
}

fn metric_value(body: &str, name: &str) -> f64 {
    for line in body.lines() {
        if line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix(name) {
            let rest = rest.trim_start();
            if rest.starts_with('{') {
                continue;
            }
            if let Some(v) = rest.split_whitespace().next()
                && let Ok(n) = v.parse::<f64>()
            {
                return n;
            }
        }
    }
    0.0
}
