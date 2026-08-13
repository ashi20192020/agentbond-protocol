use std::sync::Arc;
use std::time::Duration;

use agentbond_db::test_util::{pg_test_lock, reset_public_tables};
use agentbond_db::{Db, PgChallengeStore, PgSettlementStore};
use agentbond_payments::{
    BeginOutcome, ChallengeStore, LeaseToken, PaidDemoResult, SettlementBinding, SettlementStore,
    X402ResourceConfig, invoke_paid_demo,
};
use agentbond_payments::{MockFacilitatorClient, ResourceInfo};
use serde_json::json;

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

#[tokio::test]
async fn challenge_and_settlement_survive_restart_and_leases() {
    let (_lock, db) = setup().await;
    let challenges = PgChallengeStore::new(db.pool().clone());
    let settlements = PgSettlementStore::new(db.pool().clone());
    let fac = MockFacilitatorClient::new();
    let input = json!({"pg": 1});
    let now = 1_700_000_000i64;

    let header = {
        let first = invoke_paid_demo(&cfg(), &fac, &challenges, &settlements, None, &input, now)
            .await
            .expect("issue")
            .expect_err("402");
        // rebuild payment using issued requirements
        use agentbond_payments::{
            ExactPayloadBody, PAYMENT_REQUIRED, PaymentPayload, PaymentRequired, SCHEME_EXACT,
            SvmExactExtra, X402_VERSION,
        };
        use base64::Engine;
        let required: PaymentRequired = {
            let bytes = Engine::decode(&base64::engine::general_purpose::STANDARD, first.trim())
                .expect("b64");
            serde_json::from_slice(&bytes).expect("json")
        };
        let accepted = required.accepts[0].clone();
        let payload = PaymentPayload {
            x402_version: X402_VERSION,
            resource: ResourceInfo {
                url: cfg().resource_url,
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
                transaction: Engine::encode(&base64::engine::general_purpose::STANDARD, [42u8; 64]),
            },
            extensions: Default::default(),
        };
        let _ = PAYMENT_REQUIRED;
        Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            serde_json::to_vec(&payload).expect("json"),
        )
    };

    let paid = invoke_paid_demo(
        &cfg(),
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

    // New store instances (process restart simulation).
    let challenges2 = PgChallengeStore::new(db.pool().clone());
    let settlements2 = PgSettlementStore::new(db.pool().clone());
    let paid2 = invoke_paid_demo(
        &cfg(),
        &fac,
        &challenges2,
        &settlements2,
        Some(&header),
        &input,
        now,
    )
    .await
    .expect("retry")
    .expect("cached");
    assert_eq!(fac.settle_calls().await, 1);
    assert_eq!(paid.body, paid2.body);

    // Different input rejected.
    let err = invoke_paid_demo(
        &cfg(),
        &fac,
        &challenges2,
        &settlements2,
        Some(&header),
        &json!({"pg": 2}),
        now,
    )
    .await
    .expect_err("binding");
    assert!(
        err.to_string().contains("binding")
            || err.to_string().contains("mismatch")
            || err.to_string().contains("challenge")
            || err.to_string().contains("Invalid")
    );

    // Lease exclusivity across two stores.
    let a = PgSettlementStore::new(db.pool().clone());
    let b = PgSettlementStore::new(db.pool().clone());
    let binding = SettlementBinding {
        service_id: "hash-demo".into(),
        resource_url: "/v1/x402/services/hash-demo/invoke".into(),
        input_digest: "ab".repeat(32),
        challenge_memo: "cd".repeat(16),
    };
    let digest = "11".repeat(32);
    let first = a.begin(&digest, binding.clone()).await.expect("begin a");
    let BeginOutcome::Acquired(lease) = first else {
        panic!("expected acquired");
    };
    let second = b.begin(&digest, binding.clone()).await;
    assert!(matches!(
        second,
        Err(agentbond_payments::PaymentError::SettlementInProgress)
    ));
    let wrong = LeaseToken::new();
    assert!(
        a.complete(
            &digest,
            &binding,
            &wrong,
            PaidDemoResult {
                body: json!({}),
                payment_response_header: "x".into(),
            },
        )
        .await
        .is_err()
    );
    a.complete(
        &digest,
        &binding,
        &lease,
        PaidDemoResult {
            body: json!({"ok": true}),
            payment_response_header: "resp".into(),
        },
    )
    .await
    .expect("complete");
    let cached = b.begin(&digest, binding.clone()).await.expect("cached");
    assert!(matches!(cached, BeginOutcome::Cached(_)));

    // Expiry
    let resource = ResourceInfo {
        url: "/tmp".into(),
        description: "d".into(),
        mime_type: "application/json".into(),
    };
    let (req, challenge) = challenges
        .issue(&cfg(), &resource, &("ee".repeat(32)), now)
        .await
        .expect("issue");
    let _ = req;
    let expired = challenges
        .get_valid(&challenge.memo, challenge.expires_at + 1)
        .await;
    assert!(expired.is_err());
    let _ = Duration::from_secs(1);
}
