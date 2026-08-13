mod common;

use agentbond_types::{
    is_terminal, parse_instruction, validate_transition, ConfigAccount, JobAccount, JobState,
    ProtocolError, ProviderBondAccount, CONFIG_ACCOUNT_LEN, JOB_ACCOUNT_LEN,
    PROVIDER_BOND_ACCOUNT_LEN,
};
use common::{setup, Env, JOB_AMOUNT, MIN_BOND, START_TS};
use proptest::prelude::*;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use spl_associated_token_account_client::address::get_associated_token_address;

type JobPath = Box<dyn Fn(&mut Env, Pubkey)>;

#[test]
fn principal_conservation_settle_and_refund() {
    let paths: Vec<(&str, JobPath)> = vec![
        (
            "settle",
            Box::new(|env, job| {
                env.accept_job(&job);
                let receipt = env.make_receipt(&job, env.read_job(&job).job_nonce);
                env.submit_receipt(&job, &receipt);
                env.accept_work(&job);
            }),
        ),
        (
            "refund_funded",
            Box::new(|env, job| {
                env.set_clock(START_TS + 200);
                env.resolve_timeout_refund(&job);
            }),
        ),
        (
            "slash",
            Box::new(|env, job| {
                env.accept_job(&job);
                let receipt = env.make_receipt(&job, env.read_job(&job).job_nonce);
                env.submit_receipt(&job, &receipt);
                env.challenge_work(&job);
                env.slash_bond(&job);
            }),
        ),
    ];

    for (i, (name, run)) in paths.into_iter().enumerate() {
        let mut env = setup();
        env.bootstrap_ready();
        let nonce = 100 + i as u64;
        let job = env.create_job(nonce);
        env.fund_job(&job);
        let buyer_ata = get_associated_token_address(&env.buyer.pubkey(), &env.mint);
        let provider_ata = get_associated_token_address(&env.provider.pubkey(), &env.mint);
        let buyer_before = env.token_balance(&buyer_ata);
        let provider_before = env.token_balance(&provider_ata);
        let principal = env.read_job(&job).amount;
        assert_eq!(principal, JOB_AMOUNT);
        run(&mut env, job);
        let buyer_after = env.token_balance(&buyer_ata);
        let provider_after = env.token_balance(&provider_ata);
        let moved_to_provider = provider_after.saturating_sub(provider_before);
        let moved_to_buyer = buyer_after.saturating_sub(buyer_before);
        // Principal goes to exactly one party (slash also sends locked bond to buyer).
        if name == "settle" {
            assert_eq!(moved_to_provider, principal);
        } else if name == "refund_funded" {
            assert_eq!(moved_to_buyer, principal);
            assert_eq!(moved_to_provider, 0);
        } else if name == "slash" {
            assert_eq!(moved_to_buyer, principal + MIN_BOND);
            assert_eq!(moved_to_provider, 0);
        }
        assert_eq!(env.token_balance(&env.escrow_ata(&job)), 0);
    }
}

#[test]
fn settlement_and_refund_mutually_exclusive() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(1);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 1);
    env.submit_receipt(&job, &receipt);
    env.accept_work(&job);
    assert!(is_terminal(env.read_job(&job).state));
    let admin = env.admin.insecure_clone();
    env.send_err_code(
        &admin,
        &[env.ix_resolve_timeout_refund(&job)],
        &[],
        ProtocolError::InvalidStateTransition,
    );
}

#[test]
fn at_most_one_terminal_and_no_reentry() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(2);
    env.fund_job(&job);
    env.set_clock(START_TS + 200);
    env.resolve_timeout_refund(&job);
    let state = env.read_job(&job).state;
    assert!(is_terminal(state));
    for to in [
        JobState::Settled,
        JobState::Refunded,
        JobState::Expired,
        JobState::Slashed,
        JobState::Funded,
    ] {
        assert!(validate_transition(state, to).is_err());
    }
}

#[test]
fn repeated_instructions_cannot_duplicate_payouts() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(3);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 3);
    env.submit_receipt(&job, &receipt);
    let provider_ata = env.create_ata(&env.provider.pubkey());
    let before = env.token_balance(&provider_ata);
    env.accept_work(&job);
    assert_eq!(env.token_balance(&provider_ata), before + JOB_AMOUNT);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(
        &buyer,
        &[env.ix_accept_work(&job)],
        &[],
        ProtocolError::InvalidStateTransition,
    );
    assert_eq!(env.token_balance(&provider_ata), before + JOB_AMOUNT);
}

#[test]
fn bond_locked_never_exceeds_deposited_and_clears() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(4);
    env.fund_job(&job);
    env.accept_job(&job);
    let bond = env.read_bond();
    let job_acc = env.read_job(&job);
    assert!(bond.locked <= bond.deposited);
    assert_eq!(job_acc.locked_bond, MIN_BOND);
    assert_eq!(job_acc.locked_bond, bond.locked);
    let receipt = env.make_receipt(&job, 4);
    env.submit_receipt(&job, &receipt);
    env.accept_work(&job);
    assert_eq!(env.read_job(&job).locked_bond, 0);
    assert_eq!(env.read_bond().locked, 0);
}

#[test]
fn admin_slash_limited_to_job_locked_bond() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(5);
    env.fund_job(&job);
    env.accept_job(&job);
    let locked = env.read_job(&job).locked_bond;
    let deposited_before = env.read_bond().deposited;
    let receipt = env.make_receipt(&job, 5);
    env.submit_receipt(&job, &receipt);
    env.challenge_work(&job);
    env.slash_bond(&job);
    let bond = env.read_bond();
    assert_eq!(bond.deposited, deposited_before - locked);
    assert_eq!(bond.locked, 0);
    assert_eq!(env.read_job(&job).locked_bond, 0);
}

#[test]
fn config_unchanged_on_common_job_ops() {
    let mut env = setup();
    env.bootstrap_ready();
    let before = env.svm.get_account(&env.config_pda()).expect("config").data;
    let job = env.create_job(6);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 6);
    env.submit_receipt(&job, &receipt);
    env.accept_work(&job);
    let after = env.svm.get_account(&env.config_pda()).expect("config").data;
    assert_eq!(before, after);
}

proptest! {
    #[test]
    fn arbitrary_instruction_bytes_never_panic(data in prop::collection::vec(any::<u8>(), 0..400)) {
        let _ = parse_instruction(&data);
    }

    #[test]
    fn arbitrary_account_layout_bytes_never_panic(
        config in prop::collection::vec(any::<u8>(), 0..CONFIG_ACCOUNT_LEN + 32),
        job in prop::collection::vec(any::<u8>(), 0..JOB_ACCOUNT_LEN + 32),
        bond in prop::collection::vec(any::<u8>(), 0..PROVIDER_BOND_ACCOUNT_LEN + 32),
    ) {
        let _ = ConfigAccount::decode(&config);
        let _ = JobAccount::decode(&job);
        let _ = ProviderBondAccount::decode(&bond);
    }
}

#[test]
fn table_terminal_states_reject_fund() {
    for state in [
        JobState::Settled,
        JobState::Refunded,
        JobState::Expired,
        JobState::Slashed,
    ] {
        assert!(is_terminal(state));
        assert!(validate_transition(state, JobState::Funded).is_err());
        assert!(validate_transition(state, state).is_err());
    }
}
