mod common;

use agentbond_types::JobState;
use common::{setup, JOB_AMOUNT, MIN_BOND, START_TS};
use solana_signer::Signer;
use spl_associated_token_account_client::address::get_associated_token_address;

#[test]
fn happy_path_settle_and_close() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(1);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 1);
    let provider_ata = get_associated_token_address(&env.provider.pubkey(), &env.mint);
    let provider_before = env.token_balance(&provider_ata);
    let submit_cu = env.submit_receipt(&job, &receipt);
    let accept_cu = env.accept_work(&job);
    env.assert_job_state(&job, JobState::Settled);
    assert_eq!(
        env.token_balance(&provider_ata),
        provider_before + JOB_AMOUNT
    );
    assert_eq!(env.read_bond().locked, 0);
    env.close_job(&job);
    assert!(
        env.svm.get_account(&job).is_none()
            || env.svm.get_account(&job).unwrap().data.is_empty()
            || env.svm.get_account(&job).unwrap().lamports == 0
    );
    println!("CU submit={submit_cu} accept_work={accept_cu}");
}

#[test]
fn happy_path_timeout_refund() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(2);
    env.fund_job(&job);
    env.set_clock(START_TS + 200);
    let refund_cu = env.resolve_timeout_refund(&job);
    env.assert_job_state(&job, JobState::Refunded);
    let buyer_ata = get_associated_token_address(&env.buyer.pubkey(), &env.mint);
    assert!(env.token_balance(&buyer_ata) >= JOB_AMOUNT);
    println!("CU timeout_refund={refund_cu}");
}

#[test]
fn happy_path_challenge_timeout_settle() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(3);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 3);
    env.submit_receipt(&job, &receipt);
    let challenge_cu = env.challenge_work(&job);
    env.assert_job_state(&job, JobState::Challenged);
    env.set_clock(START_TS + 10 + 3_600);
    let settle_cu = env.resolve_timeout_settle(&job, true);
    env.assert_job_state(&job, JobState::Settled);
    println!("CU challenge={challenge_cu} challenge_timeout_settle={settle_cu}");
}

#[test]
fn happy_path_admin_slash() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(4);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 4);
    env.submit_receipt(&job, &receipt);
    env.challenge_work(&job);
    let slash_cu = env.slash_bond(&job);
    env.assert_job_state(&job, JobState::Slashed);
    assert_eq!(env.read_bond().locked, 0);
    assert!(env.read_bond().deposited <= MIN_BOND * 2 - MIN_BOND);
    let buyer_ata = get_associated_token_address(&env.buyer.pubkey(), &env.mint);
    assert!(env.token_balance(&buyer_ata) >= JOB_AMOUNT + MIN_BOND);
    println!("CU slash={slash_cu}");
}

#[test]
fn pause_blocks_entry_not_exit() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(5);
    env.fund_job(&job);
    env.accept_job(&job);
    env.set_paused(true);

    // create blocked
    let config = env.config_pda();
    let provider_pda = env.provider_pda();
    let blocked_job = env.job_pda(99);
    use agentbond_types::{encode_create_job, CreateJobPayload};
    use solana_instruction::{AccountMeta, Instruction};
    use solana_pubkey::Pubkey;
    let payload = CreateJobPayload {
        job_nonce: 99,
        amount: JOB_AMOUNT,
        request_hash: [9u8; 32],
        fund_deadline: START_TS + 100,
        accept_deadline: START_TS + 200,
        work_deadline: START_TS + 300,
        auto_settle_deadline: START_TS + 400,
    };
    let ix = Instruction {
        program_id: env.program_id,
        accounts: vec![
            AccountMeta::new(env.buyer.pubkey(), true),
            AccountMeta::new_readonly(config, false),
            AccountMeta::new_readonly(provider_pda, false),
            AccountMeta::new(blocked_job, false),
            AccountMeta::new_readonly(Pubkey::default(), false),
        ],
        data: encode_create_job(&payload).to_vec(),
    };
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(
        &buyer,
        &[ix],
        &[],
        agentbond_types::ProtocolError::ProtocolPaused,
    );

    // settle path still works
    let receipt = env.make_receipt(&job, 5);
    env.submit_receipt(&job, &receipt);
    env.accept_work(&job);
    env.assert_job_state(&job, JobState::Settled);
}

#[test]
fn double_settle_rejected() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(6);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 6);
    env.submit_receipt(&job, &receipt);
    env.accept_work(&job);
    let bond_pda = env.bond_pda();
    let escrow = get_associated_token_address(&job, &env.mint);
    let provider_ata = env.create_ata(&env.provider.pubkey());
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    use agentbond_types::{encode_empty, InstructionKind};
    use solana_instruction::{AccountMeta, Instruction};
    use spl_token::ID as TOKEN_PROGRAM_ID;
    let ix = Instruction {
        program_id: env.program_id,
        accounts: vec![
            AccountMeta::new_readonly(env.buyer.pubkey(), true),
            AccountMeta::new(job, false),
            AccountMeta::new(bond_pda, false),
            AccountMeta::new(escrow, false),
            AccountMeta::new(provider_ata, false),
            AccountMeta::new(buyer_ata, false),
            AccountMeta::new_readonly(env.mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data: encode_empty(InstructionKind::AcceptWork)
            .expect("empty")
            .to_vec(),
    };
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(
        &buyer,
        &[ix],
        &[],
        agentbond_types::ProtocolError::InvalidStateTransition,
    );
}

#[test]
fn unsolicited_escrow_dust_returned_on_settle() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(7);
    env.fund_job(&job);
    env.accept_job(&job);
    let escrow = get_associated_token_address(&job, &env.mint);
    // Donate dust directly into escrow.
    env.mint_to(&escrow, 123);
    let receipt = env.make_receipt(&job, 7);
    env.submit_receipt(&job, &receipt);
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    let buyer_before = env.token_balance(&buyer_ata);
    let provider_ata = get_associated_token_address(&env.provider.pubkey(), &env.mint);
    let provider_before = env.token_balance(&provider_ata);
    env.accept_work(&job);
    let buyer_after = env.token_balance(&buyer_ata);
    assert_eq!(
        env.token_balance(&provider_ata),
        provider_before + JOB_AMOUNT
    );
    assert!(buyer_after >= buyer_before.saturating_add(123));
    assert_eq!(env.token_balance(&escrow), 0);
}
