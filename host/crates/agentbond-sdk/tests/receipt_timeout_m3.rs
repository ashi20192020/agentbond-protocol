use agentbond_sdk::{
    ClusterKind, MAINNET_GENESIS_HASH, build_submit_receipt_plan_at, cluster_from_genesis_hash,
    job_pda, program_id, validate_plan_instructions,
};
use agentbond_types::AgentBondWorkReceiptV1;
use solana_pubkey::Pubkey;

fn sample_receipt(program: &Pubkey, job: &Pubkey, provider: &Pubkey) -> AgentBondWorkReceiptV1 {
    AgentBondWorkReceiptV1 {
        program_id: program.to_bytes(),
        genesis_hash: [7u8; 32],
        job: job.to_bytes(),
        buyer: [2u8; 32],
        provider: provider.to_bytes(),
        request_hash: [9u8; 32],
        result_hash: [4u8; 32],
        artifact_hash: [5u8; 32],
        software_hash: [6u8; 32],
        job_nonce: 1,
        created_at: 1_700_000_000,
        expires_at: 1_700_000_400,
    }
}

#[test]
fn submit_receipt_rejects_mismatches() {
    let program = program_id();
    let buyer = Pubkey::new_from_array([1u8; 32]);
    let provider = Pubkey::new_from_array([2u8; 32]);
    let job = job_pda(&program, &buyer, &provider, 1)
        .expect("job")
        .address;
    let receipt = sample_receipt(&program, &job, &provider);
    let pubkey = [3u8; 32];
    let sig = [4u8; 64];

    let mut bad_program = receipt;
    bad_program.program_id = [9u8; 32];
    assert!(
        build_submit_receipt_plan_at(
            &program,
            &job,
            &provider,
            &bad_program,
            &pubkey,
            &sig,
            Some(1_700_000_000)
        )
        .is_err()
    );

    let mut bad_job = receipt;
    bad_job.job = [8u8; 32];
    assert!(
        build_submit_receipt_plan_at(
            &program,
            &job,
            &provider,
            &bad_job,
            &pubkey,
            &sig,
            Some(1_700_000_000)
        )
        .is_err()
    );

    let mut bad_provider = receipt;
    bad_provider.provider = [7u8; 32];
    assert!(
        build_submit_receipt_plan_at(
            &program,
            &job,
            &provider,
            &bad_provider,
            &pubkey,
            &sig,
            Some(1_700_000_000)
        )
        .is_err()
    );

    assert!(
        build_submit_receipt_plan_at(
            &program,
            &job,
            &provider,
            &receipt,
            &[0u8; 32],
            &sig,
            Some(1_700_000_000)
        )
        .is_err()
    );

    assert!(
        build_submit_receipt_plan_at(
            &program,
            &job,
            &provider,
            &receipt,
            &pubkey,
            &sig,
            Some(1_700_000_401)
        )
        .is_err()
    );

    let ok = build_submit_receipt_plan_at(
        &program,
        &job,
        &provider,
        &receipt,
        &pubkey,
        &sig,
        Some(1_700_000_000),
    )
    .expect("ok");
    validate_plan_instructions(&ok, &program, 1_700_000_000).expect("plan ok");
}

#[test]
fn genesis_mainnet_detection() {
    assert_eq!(
        cluster_from_genesis_hash(MAINNET_GENESIS_HASH),
        ClusterKind::MainnetBeta
    );
}
