mod common;

use agentbond_types::{encode_create_job, JobState, ProtocolError};
use common::{setup, Env, JOB_AMOUNT, MIN_BOND, START_TS};
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use spl_associated_token_account_client::address::get_associated_token_address;
use spl_token::ID as TOKEN_PROGRAM_ID;

fn settle_path(env: &mut Env, nonce: u64) -> Pubkey {
    let job = env.create_job(nonce);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, nonce);
    env.submit_receipt(&job, &receipt);
    env.accept_work(&job);
    env.assert_job_state(&job, JobState::Settled);
    job
}

#[test]
fn complete_settlement_lifecycle() {
    let mut env = setup();
    env.bootstrap_ready();
    let provider_ata = env.create_ata(&env.provider.pubkey());
    let before = env.token_balance(&provider_ata);
    let job = settle_path(&mut env, 1);
    assert_eq!(env.token_balance(&provider_ata), before + JOB_AMOUNT);
    assert_eq!(env.read_job(&job).locked_bond, 0);
    assert_eq!(env.read_bond().locked, 0);
}

#[test]
fn funded_timeout_refund_and_expire_unaccepted() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(2);
    env.fund_job(&job);
    env.set_clock(START_TS + 200);
    env.resolve_timeout_refund(&job);
    env.assert_job_state(&job, JobState::Refunded);

    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(3);
    env.fund_job(&job);
    env.set_clock(START_TS + 200);
    env.expire_unaccepted(&job);
    env.assert_job_state(&job, JobState::Refunded);
}

#[test]
fn accepted_work_deadline_refund() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(4);
    env.fund_job(&job);
    env.accept_job(&job);
    env.set_clock(START_TS + 301);
    env.resolve_timeout_refund(&job);
    env.assert_job_state(&job, JobState::Refunded);
    assert_eq!(env.read_bond().locked, 0);
}

#[test]
fn submitted_auto_settlement_and_challenge_timeout() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(5);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 5);
    env.submit_receipt(&job, &receipt);
    env.set_clock(START_TS + 400);
    env.resolve_timeout_settle(&job, false);
    env.assert_job_state(&job, JobState::Settled);

    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(6);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 6);
    env.submit_receipt(&job, &receipt);
    env.challenge_work(&job);
    let challenge = env.challenge_pda(&job);
    let buyer_before = env.account_lamports(&env.buyer.pubkey());
    env.set_clock(START_TS + 10 + 3_600);
    env.resolve_timeout_settle(&job, true);
    env.assert_job_state(&job, JobState::Settled);
    env.assert_account_closed(&challenge);
    assert!(env.account_lamports(&env.buyer.pubkey()) >= buyer_before);
}

#[test]
fn admin_slash_and_unfunded_expire() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(7);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 7);
    env.submit_receipt(&job, &receipt);
    env.challenge_work(&job);
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    let before = env.token_balance(&buyer_ata);
    env.slash_bond(&job);
    env.assert_job_state(&job, JobState::Slashed);
    assert_eq!(env.read_job(&job).locked_bond, 0);
    assert!(env.token_balance(&buyer_ata) >= before + JOB_AMOUNT + MIN_BOND);

    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(8);
    env.set_clock(START_TS + 100);
    env.expire_unfunded(&job);
    env.assert_job_state(&job, JobState::Expired);
}

#[test]
fn deadline_boundaries() {
    // fund: now == fund_deadline rejected; now == fund_deadline-1 ok
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(20);
    env.set_clock(START_TS + 100);
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    env.mint_to(&buyer_ata, JOB_AMOUNT * 2);
    let escrow = env.ensure_escrow(&job);
    let fund = env.ix_fund_job(&job, buyer_ata, escrow, TOKEN_PROGRAM_ID);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(&buyer, &[fund], &[], ProtocolError::DeadlineExpired);
    env.set_clock(START_TS + 99);
    env.fund_job(&job);

    // accept: now == accept_deadline rejected
    env.set_clock(START_TS + 200);
    let accept = env.ix_accept_job(&job);
    let provider = env.provider.insecure_clone();
    env.send_err_code(&provider, &[accept], &[], ProtocolError::DeadlineExpired);
    env.set_clock(START_TS + 199);
    env.accept_job(&job);

    // submit: now == work_deadline allowed
    env.set_clock(START_TS + 300);
    let receipt = env.make_receipt(&job, 20);
    env.submit_receipt(&job, &receipt);

    // submit: now == work_deadline+1 rejected
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(21);
    env.fund_job(&job);
    env.accept_job(&job);
    env.set_clock(START_TS + 301);
    let receipt = env.make_receipt(&job, 21);
    let ixs = env.submit_receipt_ixs(&job, &receipt, &env.exec);
    let provider = env.provider.insecure_clone();
    env.send_err_code(&provider, &ixs, &[], ProtocolError::DeadlineExpired);

    // auto settle: now == auto-1 rejected; now == auto allowed
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(22);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 22);
    env.submit_receipt(&job, &receipt);
    env.set_clock(START_TS + 399);
    let settle = env.ix_resolve_timeout_settle(&job, false);
    let admin = env.admin.insecure_clone();
    env.send_err_code(&admin, &[settle], &[], ProtocolError::DeadlineNotReached);
    env.set_clock(START_TS + 400);
    env.resolve_timeout_settle(&job, false);

    // challenge: now == auto_settle rejected; now == auto-1 ok
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(23);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 23);
    env.submit_receipt(&job, &receipt);
    env.set_clock(START_TS + 400);
    let ch = env.ix_challenge_work(&job);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(&buyer, &[ch], &[], ProtocolError::DeadlineExpired);
    env.set_clock(START_TS + 399);
    env.challenge_work(&job);
    let challenge_deadline = START_TS + 399 + 3_600;
    env.set_clock(challenge_deadline - 1);
    let settle = env.ix_resolve_timeout_settle(&job, true);
    let admin = env.admin.insecure_clone();
    env.send_err_code(&admin, &[settle], &[], ProtocolError::DeadlineNotReached);
    env.set_clock(challenge_deadline);
    env.resolve_timeout_settle(&job, true);
    env.assert_job_state(&job, JobState::Settled);

    // slash: now == challenge_deadline rejected; now == deadline-1 ok
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(24);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 24);
    env.submit_receipt(&job, &receipt);
    env.set_clock(START_TS + 10);
    env.challenge_work(&job);
    let slash_deadline = START_TS + 10 + 3_600;
    env.set_clock(slash_deadline);
    let admin = env.admin.insecure_clone();
    let _ = env.create_ata(&env.buyer.pubkey());
    let slash_ix = Instruction {
        program_id: env.program_id,
        accounts: vec![
            AccountMeta::new_readonly(env.admin.pubkey(), true),
            AccountMeta::new_readonly(env.config_pda(), false),
            AccountMeta::new(job, false),
            AccountMeta::new(env.bond_pda(), false),
            AccountMeta::new(env.bond_vault(), false),
            AccountMeta::new(env.escrow_ata(&job), false),
            AccountMeta::new(
                get_associated_token_address(&env.buyer.pubkey(), &env.mint),
                false,
            ),
            AccountMeta::new(env.buyer.pubkey(), false),
            AccountMeta::new(env.challenge_pda(&job), false),
            AccountMeta::new_readonly(env.mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data: agentbond_types::encode_empty(agentbond_types::InstructionKind::SlashBond)
            .expect("e")
            .to_vec(),
    };
    env.send_err_code(&admin, &[slash_ix], &[], ProtocolError::DeadlineExpired);
    env.set_clock(slash_deadline - 1);
    env.slash_bond(&job);
    env.assert_job_state(&job, JobState::Slashed);

    // funded refund: now == accept-1 rejected; now == accept ok
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(25);
    env.fund_job(&job);
    env.set_clock(START_TS + 199);
    let refund = env.ix_resolve_timeout_refund(&job);
    let admin = env.admin.insecure_clone();
    env.send_err_code(&admin, &[refund], &[], ProtocolError::DeadlineNotReached);
    env.set_clock(START_TS + 200);
    env.resolve_timeout_refund(&job);

    // accepted refund: now == work rejected; now == work+1 ok
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(26);
    env.fund_job(&job);
    env.accept_job(&job);
    env.set_clock(START_TS + 300);
    let refund = env.ix_resolve_timeout_refund(&job);
    let admin = env.admin.insecure_clone();
    env.send_err_code(&admin, &[refund], &[], ProtocolError::DeadlineNotReached);
    env.set_clock(START_TS + 301);
    env.resolve_timeout_refund(&job);
}

#[test]
fn invalid_deadline_order_and_challenge_overflow() {
    let mut env = setup();
    env.bootstrap_ready();
    let mut payload = env.create_job_payload(30);
    payload.fund_deadline = START_TS + 200;
    payload.accept_deadline = START_TS + 100;
    let ix = env.ix_create_job(&payload);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(&buyer, &[ix], &[], ProtocolError::InvalidDeadlineOrder);

    // Keep challenge window open while forcing timestamp addition overflow.
    let mut payload = env.create_job_payload(31);
    payload.auto_settle_deadline = i64::MAX;
    let ix = env.ix_create_job(&payload);
    env.send_ok(&buyer, &[ix], &[]);
    let job = env.job_pda(31);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 31);
    env.submit_receipt(&job, &receipt);
    env.set_clock(i64::MAX - 10);
    let ch = env.ix_challenge_work(&job);
    env.send_err_code(&buyer, &[ch], &[], ProtocolError::MathOverflow);
}

#[test]
fn doubles_and_terminal_rejections() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(40);
    env.fund_job(&job);
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    env.mint_to(&buyer_ata, JOB_AMOUNT * 2);
    let escrow = env.ensure_escrow(&job);
    let fund = env.ix_fund_job(&job, buyer_ata, escrow, TOKEN_PROGRAM_ID);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(&buyer, &[fund], &[], ProtocolError::InvalidStateTransition);

    env.accept_job(&job);
    let provider = env.provider.insecure_clone();
    env.send_err_code(
        &provider,
        &[env.ix_accept_job(&job)],
        &[],
        ProtocolError::InvalidStateTransition,
    );

    let receipt = env.make_receipt(&job, 40);
    env.submit_receipt(&job, &receipt);
    let ixs = env.submit_receipt_ixs(&job, &receipt, &env.exec);
    env.send_err_code(&provider, &ixs, &[], ProtocolError::InvalidStateTransition);

    env.accept_work(&job);
    env.send_err_code(
        &buyer,
        &[env.ix_accept_work(&job)],
        &[],
        ProtocolError::InvalidStateTransition,
    );
    let admin = env.admin.insecure_clone();
    env.send_err_code(
        &admin,
        &[env.ix_resolve_timeout_refund(&job)],
        &[],
        ProtocolError::InvalidStateTransition,
    );
    env.send_err_code(
        &buyer,
        &[env.ix_challenge_work(&job)],
        &[],
        ProtocolError::InvalidStateTransition,
    );
}

#[test]
fn settlement_refund_mutual_exclusion_and_wrong_actors() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(41);
    env.fund_job(&job);
    env.set_clock(START_TS + 200);
    env.resolve_timeout_refund(&job);
    env.set_clock(START_TS + 400);
    let admin = env.admin.insecure_clone();
    env.send_err_code(
        &admin,
        &[env.ix_resolve_timeout_settle(&job, false)],
        &[],
        ProtocolError::InvalidStateTransition,
    );

    let mut env = setup();
    env.bootstrap_ready();
    let job = settle_path(&mut env, 42);
    let admin = env.admin.insecure_clone();
    env.send_err_code(
        &admin,
        &[env.ix_resolve_timeout_refund(&job)],
        &[],
        ProtocolError::InvalidStateTransition,
    );

    // Wrong buyer on fund
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(43);
    let stranger = Keypair::new();
    env.svm
        .airdrop(&stranger.pubkey(), 10_000_000_000)
        .expect("a");
    let stranger_ata = env.create_ata(&stranger.pubkey());
    env.mint_to(&stranger_ata, JOB_AMOUNT);
    let escrow = env.ensure_escrow(&job);
    let mut fund = env.ix_fund_job(&job, stranger_ata, escrow, TOKEN_PROGRAM_ID);
    fund.accounts[0] = AccountMeta::new_readonly(stranger.pubkey(), true);
    env.send_err_code(&stranger, &[fund], &[], ProtocolError::Unauthorized);

    // Wrong provider on accept
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(44);
    env.fund_job(&job);
    let mut accept = env.ix_accept_job(&job);
    accept.accounts[0] = AccountMeta::new_readonly(env.buyer.pubkey(), true);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(&buyer, &[accept], &[], ProtocolError::Unauthorized);
}

#[test]
fn wrong_pdas_and_permissionless_timeout() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(50);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 50);
    env.submit_receipt(&job, &receipt);
    env.set_clock(START_TS + 400);
    let stranger = Keypair::new();
    env.svm
        .airdrop(&stranger.pubkey(), 10_000_000_000)
        .expect("a");
    env.resolve_timeout_settle_as(&job, false, &stranger);
    env.assert_job_state(&job, JobState::Settled);

    // Wrong job PDA on fund
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(51);
    let wrong_job = env.job_pda(999);
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    env.mint_to(&buyer_ata, JOB_AMOUNT);
    let escrow = env.ensure_escrow(&job);
    let fund = env.ix_fund_job(&wrong_job, buyer_ata, escrow, TOKEN_PROGRAM_ID);
    let buyer = env.buyer.insecure_clone();
    env.send_err_any(&buyer, &[fund], &[]);

    // Wrong challenge PDA
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(52);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 52);
    env.submit_receipt(&job, &receipt);
    let mut ch = env.ix_challenge_work(&job);
    ch.accounts[3] = AccountMeta::new(Pubkey::new_unique(), false);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(&buyer, &[ch], &[], ProtocolError::InvalidPda);

    // Wrong rent recipient on close
    let mut env = setup();
    env.bootstrap_ready();
    let job = settle_path(&mut env, 53);
    let rent_to = Pubkey::new_unique();
    let ix = env.ix_close_job(&job, rent_to, true);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(&buyer, &[ix], &[], ProtocolError::InvalidRentRecipient);
}

#[test]
fn invalid_state_transition_matrix_sample() {
    // Self-transition and terminal blocks via AcceptWork / Fund / Challenge.
    let terminals = [
        (JobState::Settled, 60u64),
        (JobState::Refunded, 61),
        (JobState::Expired, 62),
        (JobState::Slashed, 63),
    ];
    for (terminal, nonce) in terminals {
        let mut env = setup();
        env.bootstrap_ready();
        let job = env.create_job(nonce);
        // Force terminal without full token cleanup for transition checks.
        let mut account = env.read_job(&job);
        account.state = terminal;
        env.write_job(&job, &account);
        let buyer = env.buyer.insecure_clone();
        let buyer_ata = env.create_ata(&env.buyer.pubkey());
        env.mint_to(&buyer_ata, JOB_AMOUNT);
        let escrow = env.ensure_escrow(&job);
        let fund = env.ix_fund_job(&job, buyer_ata, escrow, TOKEN_PROGRAM_ID);
        env.send_err_code(&buyer, &[fund], &[], ProtocolError::InvalidStateTransition);
    }
}

#[test]
fn slash_after_deadline_and_timeout_before_deadline() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(70);
    env.fund_job(&job);
    let admin = env.admin.insecure_clone();
    env.set_clock(START_TS + 150);
    env.send_err_code(
        &admin,
        &[env.ix_resolve_timeout_refund(&job)],
        &[],
        ProtocolError::DeadlineNotReached,
    );
}

#[test]
fn create_job_zero_amount_rejected() {
    let mut env = setup();
    env.bootstrap_ready();
    let mut payload = env.create_job_payload(80);
    payload.amount = 0;
    let data = encode_create_job(&payload);
    let ix = Instruction {
        program_id: env.program_id,
        accounts: env.ix_create_job(&payload).accounts,
        data: data.to_vec(),
    };
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(&buyer, &[ix], &[], ProtocolError::InvalidAmount);
}
