use std::sync::Arc;

use agentbond_db::test_util::{pg_test_lock, reset_public_tables};
use agentbond_db::{Db, PgChallengeStore, PgSettlementStore};
use agentbond_payments::MockFacilitatorClient;
use agentbond_payments::{
    BeginOutcome, ChallengeStore, ExactPayloadBody, LeaseToken, PaidDemoResult, PaymentError,
    PaymentPayload, PaymentRequired, ResourceInfo, SCHEME_EXACT, SettlementBinding,
    SettlementStore, SvmExactExtra, X402_VERSION, X402ResourceConfig, invoke_paid_demo, tx_digest,
};
use base64::Engine;
use serde_json::json;

/// Matches `FAIL_RETRY_SECS` in `agentbond_db::payments` (private).
const FAIL_RETRY_SECS: i64 = 2;
/// Matches `MAX_RESULT_JSON` in `agentbond_db::payments` (private).
const MAX_RESULT_JSON: usize = 16_384;

fn database_url() -> String {
    std::env::var("AGENTBOND_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://agentbond:agentbond_local_only@127.0.0.1:5433/agentbond".into()
    })
}

async fn setup() -> (std::fs::File, Arc<Db>) {
    let lock = pg_test_lock().expect("pg lock");
    let db = Arc::new(
        Db::connect(&database_url())
            .await
            .expect("connect postgres (start docker compose postgres)"),
    );
    db.migrate().await.expect("migrate");
    reset_public_tables(&db).await.expect("reset tables");
    (lock, db)
}

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

fn assert_digest_hex(label: &str, value: &str) {
    assert_eq!(
        value.len(),
        64,
        "{label} must be 64 hex chars, got len={}",
        value.len()
    );
    assert!(
        value.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
        "{label} must be lowercase hex"
    );
}

fn assert_memo_hex(label: &str, value: &str) {
    assert_eq!(
        value.len(),
        32,
        "{label} must be 32 hex chars, got len={}",
        value.len()
    );
    assert!(
        value.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
        "{label} must be lowercase hex"
    );
}

fn sample_binding() -> SettlementBinding {
    let input_digest = "ab".repeat(32);
    let challenge_memo = "cd".repeat(16);
    assert_digest_hex("input_digest", &input_digest);
    assert_memo_hex("challenge_memo", &challenge_memo);
    SettlementBinding {
        service_id: "hash-demo".into(),
        resource_url: "/v1/x402/services/hash-demo/invoke".into(),
        input_digest,
        challenge_memo,
    }
}

fn sample_tx_digest() -> String {
    let digest = "11".repeat(32);
    assert_digest_hex("tx_digest", &digest);
    digest
}

fn tx_b64() -> String {
    Engine::encode(&base64::engine::general_purpose::STANDARD, [42u8; 64])
}

/// Build a PAYMENT-SIGNATURE header from a 402 Payment-Required response body.
fn payment_header_from_402(required_header: &str, cfg: &X402ResourceConfig) -> String {
    let required: PaymentRequired = {
        let bytes = Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            required_header.trim(),
        )
        .expect("402 header base64");
        serde_json::from_slice(&bytes).expect("402 header json")
    };
    let accepted = required.accepts[0].clone();
    let memo = accepted
        .extra
        .memo
        .as_deref()
        .expect("challenge memo on requirements");
    assert_memo_hex("challenge memo from 402", memo);
    let payload = PaymentPayload {
        x402_version: X402_VERSION,
        resource: ResourceInfo {
            url: cfg.resource_url.clone(),
            description: cfg.description.clone(),
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
            transaction: tx_b64(),
        },
        extensions: Default::default(),
    };
    Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        serde_json::to_vec(&payload).expect("payment payload json"),
    )
}

async fn issue_payment_header(
    cfg: &X402ResourceConfig,
    challenges: &PgChallengeStore,
    settlements: &PgSettlementStore,
    fac: &MockFacilitatorClient,
    input: &serde_json::Value,
    now: i64,
) -> String {
    let first = invoke_paid_demo(cfg, fac, challenges, settlements, None, input, now)
        .await
        .expect("issue challenge")
        .expect_err("expected 402 payment required");
    payment_header_from_402(&first, cfg)
}

#[tokio::test]
async fn challenge_survives_restart() {
    let (_lock, db) = setup().await;
    let challenges = PgChallengeStore::new(db.pool().clone());
    let now = 1_700_000_000i64;
    let input_digest = "aa".repeat(32);
    assert_digest_hex("input_digest", &input_digest);
    let resource = ResourceInfo {
        url: cfg().resource_url.clone(),
        description: cfg().description.clone(),
        mime_type: "application/json".into(),
    };
    let (_req, challenge) = challenges
        .issue(&cfg(), &resource, &input_digest, now)
        .await
        .expect("issue challenge");
    assert_memo_hex("memo", &challenge.memo);

    let challenges2 = PgChallengeStore::new(db.pool().clone());
    let loaded = challenges2
        .get_valid(&challenge.memo, now)
        .await
        .expect("challenge must survive store restart");
    assert_eq!(loaded.memo, challenge.memo);
    assert_eq!(loaded.input_digest, input_digest);
}

#[tokio::test]
async fn challenge_expiration() {
    let (_lock, db) = setup().await;
    let challenges = PgChallengeStore::new(db.pool().clone());
    let now = 1_700_000_000i64;
    let input_digest = "bb".repeat(32);
    assert_digest_hex("input_digest", &input_digest);
    let resource = ResourceInfo {
        url: "/tmp".into(),
        description: "d".into(),
        mime_type: "application/json".into(),
    };
    let (_req, challenge) = challenges
        .issue(&cfg(), &resource, &input_digest, now)
        .await
        .expect("issue");
    assert_memo_hex("memo", &challenge.memo);

    let err = challenges
        .get_valid(&challenge.memo, challenge.expires_at + 1)
        .await
        .expect_err("expired challenge");
    assert!(matches!(err, PaymentError::ChallengeExpired));
}

#[tokio::test]
async fn exact_retry_after_restart() {
    let (_lock, db) = setup().await;
    let challenges = PgChallengeStore::new(db.pool().clone());
    let settlements = PgSettlementStore::new(db.pool().clone());
    let fac = MockFacilitatorClient::new();
    let input = json!({"pg": 1});
    let now = 1_700_000_000i64;
    let c = cfg();

    let header = issue_payment_header(&c, &challenges, &settlements, &fac, &input, now).await;
    let paid = invoke_paid_demo(
        &c,
        &fac,
        &challenges,
        &settlements,
        Some(&header),
        &input,
        now,
    )
    .await
    .expect("pay")
    .expect("200");
    assert_eq!(fac.settle_calls().await, 1);

    let challenges2 = PgChallengeStore::new(db.pool().clone());
    let settlements2 = PgSettlementStore::new(db.pool().clone());
    let paid2 = invoke_paid_demo(
        &c,
        &fac,
        &challenges2,
        &settlements2,
        Some(&header),
        &input,
        now,
    )
    .await
    .expect("retry")
    .expect("cached after restart");
    assert_eq!(fac.settle_calls().await, 1);
    assert_eq!(paid.body, paid2.body);
}

#[tokio::test]
async fn binding_mismatch_after_restart() {
    let (_lock, db) = setup().await;
    let challenges = PgChallengeStore::new(db.pool().clone());
    let settlements = PgSettlementStore::new(db.pool().clone());
    let fac = MockFacilitatorClient::new();
    let input = json!({"pg": 1});
    let now = 1_700_000_000i64;
    let c = cfg();

    let header = issue_payment_header(&c, &challenges, &settlements, &fac, &input, now).await;
    invoke_paid_demo(
        &c,
        &fac,
        &challenges,
        &settlements,
        Some(&header),
        &input,
        now,
    )
    .await
    .expect("pay")
    .expect("200");

    let challenges2 = PgChallengeStore::new(db.pool().clone());
    let settlements2 = PgSettlementStore::new(db.pool().clone());
    let err = invoke_paid_demo(
        &c,
        &fac,
        &challenges2,
        &settlements2,
        Some(&header),
        &json!({"pg": 2}),
        now,
    )
    .await
    .expect_err("binding mismatch");
    let msg = err.to_string();
    assert!(
        msg.contains("binding")
            || msg.contains("mismatch")
            || msg.contains("challenge")
            || msg.contains("Invalid")
            || matches!(err, PaymentError::BindingMismatch),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn simultaneous_lease_acquisition() {
    let (_lock, db) = setup().await;
    let a = PgSettlementStore::new(db.pool().clone());
    let b = PgSettlementStore::new(db.pool().clone());
    let binding = sample_binding();
    let digest = sample_tx_digest();

    let first = a.begin(&digest, binding.clone()).await.expect("begin a");
    let BeginOutcome::Acquired(_lease) = first else {
        panic!("expected Acquired on first store");
    };
    let second = b.begin(&digest, binding).await;
    assert!(
        matches!(second, Err(PaymentError::SettlementInProgress)),
        "second store must see InProgress, got {second:?}"
    );
}

#[tokio::test]
async fn stale_lease_recovery() {
    let (_lock, db) = setup().await;
    let a = PgSettlementStore::new(db.pool().clone());
    let b = PgSettlementStore::new(db.pool().clone());
    let binding = sample_binding();
    let digest = sample_tx_digest();

    let first = a.begin(&digest, binding.clone()).await.expect("begin a");
    assert!(matches!(first, BeginOutcome::Acquired(_)));

    // Lease expiry uses Utc::now() in payments.rs — advance via SQL (no sleep).
    sqlx::query(
        "UPDATE x402_settlements
         SET lease_expires_at = NOW() - INTERVAL '1 second'
         WHERE tx_digest = $1",
    )
    .bind(&digest)
    .execute(db.pool())
    .await
    .expect("expire lease");

    let recovered = b
        .begin(&digest, binding)
        .await
        .expect("stale lease recovery");
    assert!(
        matches!(recovered, BeginOutcome::RecoveredStale(_)),
        "other store must recover stale lease"
    );
}

#[tokio::test]
async fn wrong_lease_cannot_complete() {
    let (_lock, db) = setup().await;
    let store = PgSettlementStore::new(db.pool().clone());
    let binding = sample_binding();
    let digest = sample_tx_digest();
    let BeginOutcome::Acquired(_lease) =
        store.begin(&digest, binding.clone()).await.expect("begin")
    else {
        panic!("expected Acquired");
    };

    let err = store
        .complete(
            &digest,
            &binding,
            &LeaseToken::new(),
            PaidDemoResult {
                body: json!({}),
                payment_response_header: "x".into(),
            },
        )
        .await
        .expect_err("wrong lease");
    assert!(matches!(err, PaymentError::LeaseMismatch));
}

#[tokio::test]
async fn wrong_lease_cannot_fail() {
    let (_lock, db) = setup().await;
    let store = PgSettlementStore::new(db.pool().clone());
    let binding = sample_binding();
    let digest = sample_tx_digest();
    let BeginOutcome::Acquired(_lease) =
        store.begin(&digest, binding.clone()).await.expect("begin")
    else {
        panic!("expected Acquired");
    };

    let err = store
        .fail(&digest, &binding, &LeaseToken::new())
        .await
        .expect_err("wrong lease");
    assert!(matches!(err, PaymentError::LeaseMismatch));
}

#[tokio::test]
async fn settled_result_cannot_be_overwritten() {
    let (_lock, db) = setup().await;
    let store = PgSettlementStore::new(db.pool().clone());
    let binding = sample_binding();
    let digest = sample_tx_digest();
    let BeginOutcome::Acquired(lease) = store.begin(&digest, binding.clone()).await.expect("begin")
    else {
        panic!("expected Acquired");
    };

    let original = PaidDemoResult {
        body: json!({"ok": true, "n": 1}),
        payment_response_header: "resp-1".into(),
    };
    store
        .complete(&digest, &binding, &lease, original.clone())
        .await
        .expect("complete");

    let cached = store
        .begin(&digest, binding.clone())
        .await
        .expect("cached begin");
    match cached {
        BeginOutcome::Cached(result) => assert_eq!(result.body, original.body),
        BeginOutcome::Acquired(_) | BeginOutcome::RecoveredStale(_) => {
            panic!("settled row must not re-acquire")
        }
    }

    let overwrite = store
        .complete(
            &digest,
            &binding,
            &lease,
            PaidDemoResult {
                body: json!({"ok": false, "n": 2}),
                payment_response_header: "resp-2".into(),
            },
        )
        .await
        .expect_err("overwrite settled");
    assert!(matches!(overwrite, PaymentError::LeaseMismatch));

    let still = store.begin(&digest, binding).await.expect("still cached");
    match still {
        BeginOutcome::Cached(result) => assert_eq!(result.body, original.body),
        BeginOutcome::Acquired(_) | BeginOutcome::RecoveredStale(_) => {
            panic!("settled result overwritten")
        }
    }
}

#[tokio::test]
async fn failed_settlement_retries_after_bound() {
    let (_lock, db) = setup().await;
    let store = PgSettlementStore::new(db.pool().clone());
    let binding = sample_binding();
    let digest = sample_tx_digest();
    let BeginOutcome::Acquired(lease) = store.begin(&digest, binding.clone()).await.expect("begin")
    else {
        panic!("expected Acquired");
    };
    store.fail(&digest, &binding, &lease).await.expect("fail");

    // fail() stamps failed_at = NOW(); begin uses Utc::now() + FAIL_RETRY_SECS=2.
    // Prefer SQL over sleep: still inside the retry bound → InProgress.
    let blocked = store.begin(&digest, binding.clone()).await;
    assert!(
        matches!(blocked, Err(PaymentError::SettlementInProgress)),
        "failed settlement must block retries until FAIL_RETRY_SECS={FAIL_RETRY_SECS}, got {blocked:?}"
    );

    // payments.rs fail()/begin use Utc::now(); advance failed_at via SQL (no sleep).
    sqlx::query(
        "UPDATE x402_settlements
         SET failed_at = NOW() - make_interval(secs => $1)
         WHERE tx_digest = $2",
    )
    .bind((FAIL_RETRY_SECS + 1) as i32)
    .bind(&digest)
    .execute(db.pool())
    .await
    .expect("advance failed_at past retry bound");

    let retry = store
        .begin(&digest, binding)
        .await
        .expect("retry after FAIL_RETRY_SECS");
    assert!(
        matches!(retry, BeginOutcome::Acquired(_)),
        "must re-acquire after fail retry bound"
    );
}

#[tokio::test]
async fn raw_payment_header_and_tx_not_stored() {
    let (_lock, db) = setup().await;
    let challenges = PgChallengeStore::new(db.pool().clone());
    let settlements = PgSettlementStore::new(db.pool().clone());
    let fac = MockFacilitatorClient::new();
    let input = json!({"store": "check"});
    let now = 1_700_000_100i64;
    let c = cfg();

    let header = issue_payment_header(&c, &challenges, &settlements, &fac, &input, now).await;
    let transaction = tx_b64();
    let digest_key = tx_digest(&transaction);
    assert_digest_hex("tx_digest", &digest_key);

    invoke_paid_demo(
        &c,
        &fac,
        &challenges,
        &settlements,
        Some(&header),
        &input,
        now,
    )
    .await
    .expect("pay")
    .expect("200");

    #[derive(sqlx::FromRow)]
    struct SettlementDump {
        tx_digest: String,
        result_body: Option<serde_json::Value>,
        payment_response_header: Option<String>,
        input_digest: String,
        challenge_memo: String,
    }

    let row: SettlementDump = sqlx::query_as(
        "SELECT tx_digest, result_body, payment_response_header, input_digest, challenge_memo
         FROM x402_settlements WHERE tx_digest = $1",
    )
    .bind(&digest_key)
    .fetch_one(db.pool())
    .await
    .expect("settlement row");

    assert_digest_hex("stored tx_digest", &row.tx_digest);
    assert_digest_hex("stored input_digest", &row.input_digest);
    assert_memo_hex("stored challenge_memo", &row.challenge_memo);

    let blob = format!(
        "{}{}{}",
        row.tx_digest,
        row.payment_response_header.as_deref().unwrap_or(""),
        row.result_body
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default()
    );
    assert!(
        !blob.contains(&header),
        "raw PAYMENT-SIGNATURE header must not be persisted"
    );
    assert!(
        !blob.contains(&transaction),
        "full payment transaction must not be persisted"
    );

    let challenge_rows: Vec<(String,)> = sqlx::query_as("SELECT memo FROM x402_challenges")
        .fetch_all(db.pool())
        .await
        .expect("challenges");
    for (memo,) in &challenge_rows {
        assert_memo_hex("challenge memo", memo);
    }
    let challenge_text: String = sqlx::query_scalar(
        "SELECT COALESCE(string_agg(memo || resource_url || input_digest, ''), '')
         FROM x402_challenges",
    )
    .fetch_one(db.pool())
    .await
    .expect("challenge agg");
    assert!(
        !challenge_text.contains(&header) && !challenge_text.contains(&transaction),
        "challenges must not store raw payment header or full tx"
    );
}

#[tokio::test]
async fn oversized_result_rejected() {
    let (_lock, db) = setup().await;
    let store = PgSettlementStore::new(db.pool().clone());
    let binding = sample_binding();
    let digest = sample_tx_digest();
    let BeginOutcome::Acquired(lease) = store.begin(&digest, binding.clone()).await.expect("begin")
    else {
        panic!("expected Acquired");
    };

    let huge = "x".repeat(MAX_RESULT_JSON + 1);
    let err = store
        .complete(
            &digest,
            &binding,
            &lease,
            PaidDemoResult {
                body: json!(huge),
                payment_response_header: "resp".into(),
            },
        )
        .await
        .expect_err("oversized result");
    assert!(
        matches!(err, PaymentError::Internal(ref msg) if msg.contains("too large")),
        "expected result too large, got {err:?}"
    );

    // Lease still held; correct-sized complete should still work.
    store
        .complete(
            &digest,
            &binding,
            &lease,
            PaidDemoResult {
                body: json!({"ok": true}),
                payment_response_header: "resp".into(),
            },
        )
        .await
        .expect("complete after oversized rejection");
}

#[tokio::test]
async fn bounded_expired_challenge_cleanup() {
    let (_lock, db) = setup().await;
    let challenges = PgChallengeStore::new(db.pool().clone());
    let resource = ResourceInfo {
        url: cfg().resource_url.clone(),
        description: "cleanup".into(),
        mime_type: "application/json".into(),
    };

    // Issue with past timestamps so rows are already expired at purge time.
    for i in 0..3 {
        let digest = format!("{i:064x}");
        assert_digest_hex("input_digest", &digest);
        let issued_at = 1_600_000_000i64;
        let (_req, challenge) = challenges
            .issue(&cfg(), &resource, &digest, issued_at)
            .await
            .expect("issue expired-bound challenge");
        assert_memo_hex("memo", &challenge.memo);
        // Force expires_at into the past (issue() sets expires = issued + timeout).
        sqlx::query(
            "UPDATE x402_challenges SET expires_at = TO_TIMESTAMP($1)
             WHERE memo = $2",
        )
        .bind(issued_at + 1)
        .bind(&challenge.memo)
        .execute(db.pool())
        .await
        .expect("force expire");
    }

    let count_before: i64 = sqlx::query_scalar("SELECT COUNT(*)::bigint FROM x402_challenges")
        .fetch_one(db.pool())
        .await
        .expect("count before");
    assert!(count_before >= 3, "expected seeded challenges");

    let purged = challenges
        .purge_expired(1_700_000_000, 2)
        .await
        .expect("purge_expired");
    assert_eq!(purged, 2, "purge must honor limit");

    let count_after: i64 = sqlx::query_scalar("SELECT COUNT(*)::bigint FROM x402_challenges")
        .fetch_one(db.pool())
        .await
        .expect("count after");
    assert_eq!(count_after, count_before - 2);

    let purged_rest = challenges
        .purge_expired(1_700_000_000, 64)
        .await
        .expect("purge remainder");
    assert!(purged_rest >= 1);
}
