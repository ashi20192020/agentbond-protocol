//! Milestone 3 app catalog + plan builder smoke tests.

use agentbond_app::{
    AcceptJobRequest, AcceptWorkRequest, AppConfig, ChallengeRequest, CreateJobRequest,
    FundJobRequest, ServiceCatalog, ServiceEntry, build_accept_job_plan, build_accept_work_plan,
    build_challenge_plan, build_create_job_plan, build_fund_job_plan,
};
use agentbond_sdk::{AccountData, ChainReader, MockChainReader, job_pda, program_id};
use agentbond_types::{JobAccount, JobState};
use solana_pubkey::Pubkey;

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
        max_request_bytes: 65536,
        bind_address: "127.0.0.1:8080".into(),
        catalog_path: "config/example.catalog.json".into(),
    }
}

fn valid_entry(id: &str, provider: &str) -> ServiceEntry {
    ServiceEntry {
        service_id: id.into(),
        provider: provider.into(),
        name: "Demo".into(),
        description: "A demo service".into(),
        request_schema: "demo.v1".into(),
        x402_demo_route: None,
    }
}

#[test]
fn catalog_validation() {
    let ok = ServiceCatalog::from_entries(vec![valid_entry(
        "hash-demo",
        "11111111111111111111111111111113",
    )])
    .expect("valid catalog");
    assert_eq!(ok.list().len(), 1);
    assert_eq!(ok.get("hash-demo").expect("get").name, "Demo");

    assert!(
        ServiceCatalog::from_entries(vec![valid_entry("", "11111111111111111111111111111113",)])
            .is_err()
    );

    assert!(
        ServiceCatalog::from_entries(vec![valid_entry("bad-provider", "not-a-pubkey",)]).is_err()
    );

    assert!(
        ServiceCatalog::from_entries(vec![
            valid_entry("dup", "11111111111111111111111111111113"),
            valid_entry("dup", "11111111111111111111111111111114"),
        ])
        .is_err()
    );

    let mut long = valid_entry("long", "11111111111111111111111111111113");
    long.name = "x".repeat(65);
    assert!(ServiceCatalog::from_entries(vec![long]).is_err());
}

#[tokio::test]
async fn plan_builder_smoke_with_mock_reader() {
    let cfg = test_config();
    cfg.validate().expect("config");
    let reader = MockChainReader::new();
    reader.set_timestamp(1_700_000_000).await;

    let buyer = "11111111111111111111111111111111";
    let provider = "11111111111111111111111111111112";
    let create = CreateJobRequest {
        buyer: buyer.into(),
        provider: provider.into(),
        job_nonce: 1,
        amount: 1000,
        request_hash_hex: "09".repeat(32),
        fund_deadline: 1_700_000_100,
        accept_deadline: 1_700_000_200,
        work_deadline: 1_700_000_300,
        auto_settle_deadline: 1_700_000_400,
    };
    let plan = build_create_job_plan(&cfg, &reader, &create)
        .await
        .expect("create plan");
    assert_eq!(plan.action, "create_job");
    assert!(!plan.instructions.is_empty());
    assert_eq!(plan.required_signers, vec![buyer.to_string()]);

    let fund = build_fund_job_plan(
        &cfg,
        &FundJobRequest {
            buyer: buyer.into(),
            provider: provider.into(),
            job_nonce: 1,
        },
    )
    .expect("fund");
    assert_eq!(fund.action, "fund_job");

    let accept = build_accept_job_plan(
        &cfg,
        &AcceptJobRequest {
            buyer: buyer.into(),
            provider: provider.into(),
            job_nonce: 1,
        },
    )
    .expect("accept");
    assert_eq!(accept.action, "accept_job");

    let accept_work = build_accept_work_plan(
        &cfg,
        &AcceptWorkRequest {
            buyer: buyer.into(),
            provider: provider.into(),
            job_nonce: 1,
        },
    )
    .expect("accept work");
    assert_eq!(accept_work.action, "accept_work");

    let challenge = build_challenge_plan(
        &cfg,
        &ChallengeRequest {
            buyer: buyer.into(),
            provider: provider.into(),
            job_nonce: 1,
            reason_hash_hex: "0a".repeat(32),
        },
    )
    .expect("challenge");
    assert_eq!(challenge.action, "challenge_work");

    // Seed a job account so timeout resolution can be exercised later if needed.
    let program = program_id();
    let buyer_pk: Pubkey = buyer.parse().expect("buyer");
    let provider_pk: Pubkey = provider.parse().expect("provider");
    let job_addr = job_pda(&program, &buyer_pk, &provider_pk, 1)
        .expect("job pda")
        .address;
    let job = JobAccount {
        bump: 255,
        state: JobState::Created,
        buyer: buyer_pk.to_bytes(),
        provider: provider_pk.to_bytes(),
        mint: [0u8; 32],
        token_program: TOKEN_PROGRAM
            .parse::<Pubkey>()
            .expect("token program")
            .to_bytes(),
        amount: 1000,
        job_nonce: 1,
        fund_deadline: 1_700_000_050,
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
    assert!(reader.get_account(&job_addr).await.expect("get").is_some());
}
