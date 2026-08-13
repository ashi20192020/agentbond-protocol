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

fn framed_logs(program: &str, kinds: &[(ProtocolEventKind, u64)]) -> Vec<String> {
    let mut logs = vec![format!("Program {program} invoke [1]")];
    for (kind, amount) in kinds {
        logs.push(format!("Program data: {}", event_b64(*kind, *amount)));
    }
    logs.push(format!("Program {program} success"));
    logs
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
    let logs = framed_logs(
        &program,
        &[
            (ProtocolEventKind::JobCreated, 5),
            (ProtocolEventKind::JobFunded, 5),
        ],
    );
    let json = serde_json::json!({
        "program_id": program,
        "updates": [
            {"type":"slot","slot":10,"parent_slot":null,"status":"processed","block_time":1700000000},
            {"type":"transaction","signature":"B".repeat(64),"slot":10,"commitment":"processed","logs": logs},
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
    let rendered = metrics.render().expect("metrics encode");
    assert!(rendered.contains("agentbond_checkpoint_slot"));
    assert!(
        metrics.decoded_events.get() >= 2,
        "decode counters must move during fixture replay"
    );
    assert_eq!(
        metrics.checkpoint_slot.get(),
        i64::try_from(finalized).expect("slot fits i64")
    );

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
        gaps.iter().any(|(f, t, _)| *f == 16 && *t == 19),
        "expected gap 16..=19, got {gaps:?}"
    );
}

#[tokio::test]
async fn processed_projection_survives_engine_restart() {
    let (_lock, db) = db().await;
    let metrics = IndexerMetrics::new().expect("metrics");
    let program = agentbond_sdk::program_id().to_string();
    let job = bs58::encode([9u8; 32]).into_string();
    // Minimal job account bytes (Funded) owned by program — address not PDA-checked.
    let job_b64 = {
        let mut out = vec![0u8; 253];
        out[0] = 4;
        out[1] = 1;
        out[2] = 255;
        out[3] = 1;
        out[4..36].copy_from_slice(&[1u8; 32]);
        out[36..68].copy_from_slice(&[2u8; 32]);
        out[68..100].copy_from_slice(&[3u8; 32]);
        out[100..132].copy_from_slice(&[4u8; 32]);
        out[132..140].copy_from_slice(&1000u64.to_le_bytes());
        out[140..148].copy_from_slice(&7u64.to_le_bytes());
        for (off, v) in [
            (148, 1_700_000_100i64),
            (156, 1_700_000_200),
            (164, 1_700_000_300),
            (172, 1_700_000_400),
        ] {
            out[off..off + 8].copy_from_slice(&v.to_le_bytes());
        }
        out[252] = 6;
        Engine::encode(&base64::engine::general_purpose::STANDARD, out)
    };
    let processed = serde_json::json!({
        "program_id": program,
        "updates": [
            {"type":"slot","slot":50,"parent_slot":null,"status":"processed","block_time":1},
            {"type":"account","address": job, "slot":50,"write_version":1,"owner": program,
             "lamports":1,"deleted":false,"commitment":"processed","data_base64": job_b64}
        ]
    });
    IndexerEngine::new(db.clone(), metrics.clone())
        .run_source(&FixtureSource::from_json(&processed.to_string()).expect("p"))
        .await
        .expect("processed ingest");

    // New engine instance before finalization.
    let finalize = serde_json::json!({
        "program_id": program,
        "updates": [
            {"type":"slot","slot":50,"parent_slot":null,"status":"finalized","block_time":1}
        ]
    });
    IndexerEngine::new(db.clone(), metrics)
        .run_source(&FixtureSource::from_json(&finalize.to_string()).expect("f"))
        .await
        .expect("finalize after restart");

    let reads = ReadRepo::new(db.pool().clone());
    let jobs = reads
        .list_jobs(20, None, None, None, None)
        .await
        .expect("jobs");
    assert_eq!(jobs.items.len(), 1);
    assert_eq!(jobs.items[0].address, job);
}

#[tokio::test]
async fn processed_events_are_not_public_until_finalized() {
    let (_lock, db) = db().await;
    let metrics = IndexerMetrics::new().expect("metrics");
    let program = agentbond_sdk::program_id().to_string();
    let logs = framed_logs(&program, &[(ProtocolEventKind::JobCreated, u64::MAX)]);
    let json = serde_json::json!({
        "program_id": program,
        "updates": [
            {"type":"slot","slot":100,"parent_slot":null,"status":"processed","block_time":1},
            {"type":"transaction","signature":"C".repeat(64),"slot":100,"commitment":"processed","logs": logs}
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
    let logs = framed_logs(&program, &[(ProtocolEventKind::JobCreated, 1)]);
    let json = serde_json::json!({
        "program_id": program,
        "updates": [
            {"type":"slot","slot":200,"parent_slot":null,"status":"processed","block_time":1},
            {"type":"transaction","signature":"D".repeat(64),"slot":200,"commitment":"processed","logs": logs},
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
async fn finalized_cannot_downgrade_and_conflicting_ancestry() {
    let (_lock, db) = db().await;
    let repo = ProjectionRepo::new(db.pool().clone());
    repo.upsert_slot(&SlotUpdate {
        slot: 300,
        parent_slot: None,
        status: Commitment::Finalized,
        block_time: None,
    })
    .await
    .expect("finalized");
    repo.upsert_slot(&SlotUpdate {
        slot: 300,
        parent_slot: None,
        status: Commitment::Processed,
        block_time: None,
    })
    .await
    .expect("downgrade ignored");
    let status: (String,) = sqlx::query_as("SELECT status FROM indexer_slots WHERE slot = 300")
        .fetch_one(db.pool())
        .await
        .expect("status");
    assert_eq!(
        status.0, "finalized",
        "finalized must not downgrade to processed"
    );

    let err = repo
        .upsert_slot(&SlotUpdate {
            slot: 300,
            parent_slot: None,
            status: Commitment::Dead,
            block_time: None,
        })
        .await
        .expect_err("finalized -> dead");
    assert!(err.to_string().contains("finalized"));

    repo.upsert_slot(&SlotUpdate {
        slot: 310,
        parent_slot: None,
        status: Commitment::Dead,
        block_time: None,
    })
    .await
    .expect("dead parent");
    repo.upsert_slot(&SlotUpdate {
        slot: 311,
        parent_slot: Some(310),
        status: Commitment::Processed,
        block_time: None,
    })
    .await
    .expect("child");
    let err = repo.finalize_slot(311).await.expect_err("conflict");
    assert!(err.to_string().contains("ancestry") || err.to_string().contains("dead"));
}

#[tokio::test]
async fn conflicting_parents_are_rejected() {
    let (_lock, db) = db().await;
    let repo = ProjectionRepo::new(db.pool().clone());

    repo.upsert_slot(&SlotUpdate {
        slot: 700,
        parent_slot: Some(699),
        status: Commitment::Processed,
        block_time: None,
    })
    .await
    .expect("processed");
    let err = repo
        .upsert_slot(&SlotUpdate {
            slot: 700,
            parent_slot: Some(698),
            status: Commitment::Processed,
            block_time: None,
        })
        .await
        .expect_err("processed parent conflict");
    assert!(err.to_string().contains("parent"));

    repo.upsert_slot(&SlotUpdate {
        slot: 701,
        parent_slot: Some(700),
        status: Commitment::Confirmed,
        block_time: None,
    })
    .await
    .expect("confirmed");
    let err = repo
        .upsert_slot(&SlotUpdate {
            slot: 701,
            parent_slot: Some(699),
            status: Commitment::Finalized,
            block_time: None,
        })
        .await
        .expect_err("confirmed→finalized parent conflict");
    assert!(err.to_string().contains("parent"));

    repo.upsert_slot(&SlotUpdate {
        slot: 702,
        parent_slot: None,
        status: Commitment::Finalized,
        block_time: None,
    })
    .await
    .expect("finalized none");
    let err = repo
        .upsert_slot(&SlotUpdate {
            slot: 702,
            parent_slot: Some(701),
            status: Commitment::Finalized,
            block_time: None,
        })
        .await
        .expect_err("finalized parent fill rejected");
    assert!(err.to_string().contains("parent") || err.to_string().contains("finalization"));
}

#[tokio::test]
async fn missing_parent_and_cycle_rejected_on_finalize() {
    let (_lock, db) = db().await;
    let repo = ProjectionRepo::new(db.pool().clone());

    repo.upsert_slot(&SlotUpdate {
        slot: 800,
        parent_slot: Some(799),
        status: Commitment::Processed,
        block_time: None,
    })
    .await
    .expect("child with missing parent");
    let err = repo.finalize_slot(800).await.expect_err("missing parent");
    assert!(err.to_string().contains("missing parent") || err.to_string().contains("ancestry"));

    // Create a two-slot cycle via direct SQL (upsert rejects self-parent).
    sqlx::query(
        "INSERT INTO indexer_slots (slot, parent_slot, status, updated_at)
         VALUES (810, 811, 'processed', NOW()), (811, 810, 'processed', NOW())
         ON CONFLICT (slot) DO NOTHING",
    )
    .execute(db.pool())
    .await
    .expect("cycle seed");
    let err = repo.finalize_slot(810).await.expect_err("cycle");
    assert!(err.to_string().contains("cycle") || err.to_string().contains("ancestry"));
}

#[tokio::test]
async fn out_of_order_slots_do_not_create_false_gaps() {
    let (_lock, db) = db().await;
    let metrics = IndexerMetrics::new().expect("metrics");
    let program = agentbond_sdk::program_id().to_string();
    let json = serde_json::json!({
        "program_id": program,
        "updates": [
            {"type":"slot","slot":20,"parent_slot":null,"status":"processed","block_time":1},
            {"type":"slot","slot":18,"parent_slot":null,"status":"processed","block_time":1},
            {"type":"slot","slot":20,"parent_slot":null,"status":"confirmed","block_time":1}
        ]
    });
    IndexerEngine::new(db.clone(), metrics)
        .run_source(&FixtureSource::from_json(&json.to_string()).expect("f"))
        .await
        .expect("run");
    let gaps = ProjectionRepo::new(db.pool().clone())
        .open_gaps()
        .await
        .expect("gaps");
    assert!(gaps.is_empty(), "false gaps: {gaps:?}");
}

#[tokio::test]
async fn successful_and_failed_backfill_are_recorded() {
    let (_lock, db) = db().await;
    let metrics = IndexerMetrics::new().expect("metrics");
    let program = agentbond_sdk::program_id().to_string();

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
    assert!(
        open.iter()
            .any(|(f, t, s)| *f == 401 && *t == 402 && s != "repaired")
    );

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
        .with_backfill(Arc::new(MapBackfill {
            slots: map,
            accounts_reconciled: false,
        }))
        .run_source(&ok_src)
        .await
        .expect("ok run");
    let open2 = repo.open_gaps().await.expect("gaps2");
    assert!(
        open2
            .iter()
            .any(|(f, t, s)| *f == 501 && *t == 502 && s == "partial"),
        "expected partial gap, got {open2:?}"
    );
    let _ = NullBackfill;
}
