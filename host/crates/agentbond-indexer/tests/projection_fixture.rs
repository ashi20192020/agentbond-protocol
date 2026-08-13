use std::collections::HashMap;
use std::sync::Arc;

use agentbond_db::test_util::{pg_test_lock, reset_public_tables};
use agentbond_db::{Commitment, Db, ProjectionRepo, ReadRepo, SlotUpdate};
use agentbond_indexer::{FixtureSource, IndexerEngine, IndexerMetrics, MapBackfill, NullBackfill};
use agentbond_types::{ProtocolEvent, ProtocolEventKind};
use base64::Engine;

fn database_url() -> String {
    std::env::var("AGENTBOND_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://agentbond:agentbond_local_only@127.0.0.1:5433/agentbond".into()
    })
}

fn event_b64(kind: ProtocolEventKind, amount: u64) -> String {
    let event = ProtocolEvent {
        kind,
        subject: [9u8; 32],
        actor: [8u8; 32],
        amount,
        timestamp: 1_700_000_000,
    };
    Engine::encode(&base64::engine::general_purpose::STANDARD, event.encode())
}

async fn db() -> (std::fs::File, Arc<Db>) {
    let lock = pg_test_lock().expect("pg lock");
    let db = Arc::new(
        Db::connect(&database_url())
            .await
            .expect("postgres required on 5433 (docker compose up -d postgres)"),
    );
    db.migrate().await.expect("migrate");
    reset_public_tables(&db).await.expect("reset");
    (lock, db)
}

#[tokio::test]
async fn fixture_replay_finalizes_and_is_idempotent() {
    let (_lock, db) = db().await;
    let metrics = IndexerMetrics::new().expect("metrics");
    let program = agentbond_sdk::program_id().to_string();
    let json = serde_json::json!({
        "program_id": program,
        "updates": [
            {"type":"slot","slot":10,"parent_slot":null,"status":"processed","block_time":1700000000},
            {"type":"transaction","signature":"B".repeat(64),"slot":10,"commitment":"processed","logs":[
                format!("Program data: {}", event_b64(ProtocolEventKind::JobCreated, 5)),
                format!("Program data: {}", event_b64(ProtocolEventKind::JobFunded, 5))
            ]},
            {"type":"slot","slot":10,"parent_slot":null,"status":"finalized","block_time":1700000000},
            {"type":"slot","slot":12,"parent_slot":10,"status":"processed","block_time":1700000002},
            {"type":"slot","slot":12,"parent_slot":10,"status":"finalized","block_time":1700000002}
        ]
    });
    let source = FixtureSource::from_json(&json.to_string()).expect("fixture");
    let engine = IndexerEngine::new(db.clone(), metrics.clone());
    engine.run_source(&source).await.expect("run1");
    engine
        .run_source(&FixtureSource::from_json(&json.to_string()).expect("fixture2"))
        .await
        .expect("run2 idempotent");

    let repo = ProjectionRepo::new(db.pool().clone());
    let (finalized, _) = repo.checkpoint().await.expect("checkpoint");
    assert!(finalized >= 12);
    let reads = ReadRepo::new(db.pool().clone());
    let status = reads.status().await.expect("status");
    assert_eq!(status.as_of_slot, finalized.to_string());

    let gap_source = FixtureSource::from_json(
        &serde_json::json!({
            "program_id": program,
            "updates": [
                {"type":"slot","slot":15,"parent_slot":12,"status":"processed","block_time":1},
                {"type":"slot","slot":20,"parent_slot":15,"status":"processed","block_time":2},
                {"type":"slot","slot":20,"parent_slot":15,"status":"finalized","block_time":2}
            ]
        })
        .to_string(),
    )
    .expect("gap fixture");
    let engine2 = IndexerEngine::new(db.clone(), metrics);
    engine2.run_source(&gap_source).await.expect("gap run");
    let gaps = repo.open_gaps().await.expect("gaps");
    assert!(
        gaps.iter().any(|(f, t)| *f == 16 && *t == 19),
        "expected gap 16..=19, got {gaps:?}"
    );
}

#[tokio::test]
async fn processed_accounts_are_not_public_until_finalized() {
    let (_lock, db) = db().await;
    let metrics = IndexerMetrics::new().expect("metrics");
    let program = agentbond_sdk::program_id().to_string();
    // only processed slot + account; no finalize
    let json = serde_json::json!({
        "program_id": program,
        "updates": [
            {"type":"slot","slot":100,"parent_slot":null,"status":"processed","block_time":1},
            {"type":"transaction","signature":"C".repeat(64),"slot":100,"commitment":"processed","logs":[
                format!("Program data: {}", event_b64(ProtocolEventKind::JobCreated, u64::MAX))
            ]}
        ]
    });
    IndexerEngine::new(db.clone(), metrics)
        .run_source(&FixtureSource::from_json(&json.to_string()).expect("f"))
        .await
        .expect("run");
    let reads = ReadRepo::new(db.pool().clone());
    let hist = reads
        .job_history(&bs58::encode([9u8; 32]).into_string(), 10, None)
        .await
        .expect("hist");
    assert!(hist.items.is_empty(), "processed events must stay private");
}

#[tokio::test]
async fn dead_fork_cleans_non_finalized_staging() {
    let (_lock, db) = db().await;
    let metrics = IndexerMetrics::new().expect("metrics");
    let program = agentbond_sdk::program_id().to_string();
    let json = serde_json::json!({
        "program_id": program,
        "updates": [
            {"type":"slot","slot":200,"parent_slot":null,"status":"processed","block_time":1},
            {"type":"transaction","signature":"D".repeat(64),"slot":200,"commitment":"processed","logs":[
                format!("Program data: {}", event_b64(ProtocolEventKind::JobCreated, 1))
            ]},
            {"type":"slot","slot":200,"parent_slot":null,"status":"dead","block_time":1}
        ]
    });
    IndexerEngine::new(db.clone(), metrics)
        .run_source(&FixtureSource::from_json(&json.to_string()).expect("f"))
        .await
        .expect("run");
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*)::bigint FROM raw_protocol_events WHERE slot = 200")
            .fetch_one(db.pool())
            .await
            .expect("count");
    assert_eq!(count.0, 0);
}

#[tokio::test]
async fn conflicting_finalized_ancestry_is_hard_error() {
    let (_lock, db) = db().await;
    let repo = ProjectionRepo::new(db.pool().clone());
    repo.upsert_slot(&SlotUpdate {
        slot: 300,
        parent_slot: None,
        status: Commitment::Dead,
        block_time: None,
    })
    .await
    .expect("dead parent");
    repo.upsert_slot(&SlotUpdate {
        slot: 301,
        parent_slot: Some(300),
        status: Commitment::Processed,
        block_time: None,
    })
    .await
    .expect("child");
    let err = repo.finalize_slot(301, &[]).await.expect_err("conflict");
    assert!(err.to_string().contains("ancestry") || err.to_string().contains("dead"));
}

#[tokio::test]
async fn successful_and_failed_backfill_are_recorded() {
    let (_lock, db) = db().await;
    let metrics = IndexerMetrics::new().expect("metrics");
    let program = agentbond_sdk::program_id().to_string();

    // failed backfill (NullBackfill default)
    let fail_src = FixtureSource::from_json(
        &serde_json::json!({
            "program_id": program,
            "updates": [
                {"type":"slot","slot":400,"parent_slot":null,"status":"processed","block_time":1},
                {"type":"slot","slot":403,"parent_slot":400,"status":"processed","block_time":2}
            ]
        })
        .to_string(),
    )
    .expect("fail fixture");
    IndexerEngine::new(db.clone(), metrics.clone())
        .run_source(&fail_src)
        .await
        .expect("fail run");
    let repo = ProjectionRepo::new(db.pool().clone());
    let open = repo.open_gaps().await.expect("gaps");
    assert!(open.iter().any(|(f, t)| *f == 401 && *t == 402));

    // successful backfill with MapBackfill
    let mut map = HashMap::new();
    map.insert(
        501,
        vec![agentbond_indexer::ChainUpdate::Slot(SlotUpdate {
            slot: 501,
            parent_slot: Some(500),
            status: Commitment::Processed,
            block_time: Some(1),
        })],
    );
    map.insert(
        502,
        vec![agentbond_indexer::ChainUpdate::Slot(SlotUpdate {
            slot: 502,
            parent_slot: Some(501),
            status: Commitment::Processed,
            block_time: Some(2),
        })],
    );
    let ok_src = FixtureSource::from_json(
        &serde_json::json!({
            "program_id": program,
            "updates": [
                {"type":"slot","slot":500,"parent_slot":null,"status":"processed","block_time":1},
                {"type":"slot","slot":503,"parent_slot":502,"status":"processed","block_time":3}
            ]
        })
        .to_string(),
    )
    .expect("ok fixture");
    IndexerEngine::new(db.clone(), metrics)
        .with_backfill(Arc::new(MapBackfill { slots: map }))
        .run_source(&ok_src)
        .await
        .expect("ok run");
    let open2 = repo.open_gaps().await.expect("gaps2");
    assert!(
        !open2.iter().any(|(f, t)| *f == 501 && *t == 502),
        "repaired gap should leave open/failed set"
    );
    let _ = NullBackfill;
}
