mod common;

use agentbond_types::{JobState, ProtocolError};
use common::{setup, JOB_AMOUNT, MIN_BOND, START_TS};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

#[test]
fn close_settled_refunded_expired_slashed() {
    // Settled
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(1);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 1);
    env.submit_receipt(&job, &receipt);
    env.accept_work(&job);
    let buyer_before = env.account_lamports(&env.buyer.pubkey());
    let job_lamports = env.account_lamports(&job);
    env.close_job(&job);
    env.assert_account_closed(&job);
    assert!(env.account_lamports(&env.buyer.pubkey()) >= buyer_before + job_lamports - 10_000);

    // Refunded
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(2);
    env.fund_job(&job);
    env.set_clock(START_TS + 200);
    env.resolve_timeout_refund(&job);
    env.close_job(&job);
    env.assert_account_closed(&job);

    // Expired
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(3);
    env.set_clock(START_TS + 100);
    env.expire_unfunded(&job);
    env.close_job(&job);
    env.assert_account_closed(&job);

    // Slashed
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(4);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 4);
    env.submit_receipt(&job, &receipt);
    env.challenge_work(&job);
    let challenge = env.challenge_pda(&job);
    let buyer_before = env.account_lamports(&env.buyer.pubkey());
    env.slash_bond(&job);
    env.assert_account_closed(&challenge);
    assert!(env.account_lamports(&env.buyer.pubkey()) > buyer_before);
    env.close_job(&job);
    env.assert_account_closed(&job);
}

#[test]
fn reject_closing_nonterminal_and_wrong_buyer() {
    let states = [
        JobState::Created,
        JobState::Funded,
        JobState::Accepted,
        JobState::Submitted,
        JobState::Challenged,
    ];
    for (i, state) in states.into_iter().enumerate() {
        let mut env = setup();
        env.bootstrap_ready();
        let job = env.create_job(10 + i as u64);
        if state != JobState::Created {
            // Advance as far as needed for account existence; then force state.
            if matches!(
                state,
                JobState::Funded | JobState::Accepted | JobState::Submitted | JobState::Challenged
            ) {
                env.fund_job(&job);
            }
            if matches!(
                state,
                JobState::Accepted | JobState::Submitted | JobState::Challenged
            ) {
                env.accept_job(&job);
            }
            if matches!(state, JobState::Submitted | JobState::Challenged) {
                let receipt = env.make_receipt(&job, 10 + i as u64);
                env.submit_receipt(&job, &receipt);
            }
            if state == JobState::Challenged {
                env.challenge_work(&job);
            }
            let mut job_acc = env.read_job(&job);
            job_acc.state = state;
            env.write_job(&job, &job_acc);
        }
        let buyer = env.buyer.insecure_clone();
        let include = env.svm.get_account(&env.escrow_ata(&job)).is_some();
        let ix = env.ix_close_job(&job, env.buyer.pubkey(), include);
        env.send_err_code(&buyer, &[ix], &[], ProtocolError::InvalidStateTransition);
    }

    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(20);
    env.set_clock(START_TS + 100);
    env.expire_unfunded(&job);
    let stranger = Keypair::new();
    env.svm
        .airdrop(&stranger.pubkey(), 10_000_000_000)
        .expect("a");
    let mut ix = env.ix_close_job(&job, stranger.pubkey(), false);
    ix.accounts[0] = solana_instruction::AccountMeta::new_readonly(stranger.pubkey(), true);
    env.send_err_code(&stranger, &[ix], &[], ProtocolError::Unauthorized);
}

#[test]
fn reject_wrong_rent_recipient_and_nonzero_escrow() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(30);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 30);
    env.submit_receipt(&job, &receipt);
    env.accept_work(&job);
    let wrong = Pubkey::new_unique();
    let ix = env.ix_close_job(&job, wrong, true);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(&buyer, &[ix], &[], ProtocolError::InvalidRentRecipient);

    // Nonzero escrow blocks close
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(31);
    env.set_clock(START_TS + 100);
    env.expire_unfunded(&job);
    let escrow = env.ensure_escrow(&job);
    env.mint_to(&escrow, 5);
    let ix = env.ix_close_job(&job, env.buyer.pubkey(), true);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(&buyer, &[ix], &[], ProtocolError::EscrowNotEmpty);
}

#[test]
fn challenge_rent_returns_on_timeout_settle() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(40);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 40);
    env.submit_receipt(&job, &receipt);
    env.challenge_work(&job);
    let challenge = env.challenge_pda(&job);
    let challenge_lamports = env.account_lamports(&challenge);
    let buyer_before = env.account_lamports(&env.buyer.pubkey());
    env.set_clock(START_TS + 10 + 3_600);
    env.resolve_timeout_settle(&job, true);
    env.assert_account_closed(&challenge);
    assert!(env.account_lamports(&env.buyer.pubkey()) >= buyer_before + challenge_lamports - 5_000);
}

#[test]
fn repeated_close_cannot_transfer_rent_twice() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(50);
    env.set_clock(START_TS + 100);
    env.expire_unfunded(&job);
    env.close_job(&job);
    env.assert_account_closed(&job);
    let buyer = env.buyer.insecure_clone();
    let ix = env.ix_close_job(&job, env.buyer.pubkey(), false);
    env.send_err_any(&buyer, &[ix], &[]);
    let _ = (JOB_AMOUNT, MIN_BOND);
}
