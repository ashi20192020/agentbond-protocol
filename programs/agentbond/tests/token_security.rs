mod common;

use agentbond_types::{encode_empty, InstructionKind, ProtocolError};
use common::{setup, token_2022_id, JOB_AMOUNT, MIN_BOND};
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use spl_associated_token_account_client::address::get_associated_token_address;
use spl_token::ID as TOKEN_PROGRAM_ID;

#[test]
fn legacy_token_success_and_token2022_rejection() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(1);
    env.fund_job(&job);
    env.assert_job_state(&job, agentbond_types::JobState::Funded);

    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(2);
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    env.mint_to(&buyer_ata, JOB_AMOUNT * 2);
    let escrow = env.ensure_escrow(&job);
    let fund = env.ix_fund_job(&job, buyer_ata, escrow, token_2022_id());
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(&buyer, &[fund], &[], ProtocolError::InvalidTokenProgram);

    let arbitrary = Pubkey::new_unique();
    let fund = env.ix_fund_job(&job, buyer_ata, escrow, arbitrary);
    env.send_err_code(&buyer, &[fund], &[], ProtocolError::InvalidTokenProgram);
}

#[test]
fn wrong_mint_accounts_and_authorities() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(3);
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    env.mint_to(&buyer_ata, JOB_AMOUNT * 2);
    let escrow = env.ensure_escrow(&job);
    let mut fund = env.ix_fund_job(&job, buyer_ata, escrow, TOKEN_PROGRAM_ID);
    fund.accounts[5] = AccountMeta::new_readonly(Pubkey::new_unique(), false);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(&buyer, &[fund], &[], ProtocolError::InvalidMint);

    // Wrong buyer token account (provider ATA)
    let provider_ata = env.create_ata(&env.provider.pubkey());
    env.mint_to(&provider_ata, JOB_AMOUNT);
    let fund = env.ix_fund_job(&job, provider_ata, escrow, TOKEN_PROGRAM_ID);
    env.send_err_code(
        &buyer,
        &[fund],
        &[],
        ProtocolError::InvalidTokenAccountAuthority,
    );

    // Wrong escrow ATA (buyer ATA as escrow)
    let fund = env.ix_fund_job(&job, buyer_ata, buyer_ata, TOKEN_PROGRAM_ID);
    env.send_err_code(
        &buyer,
        &[fund],
        &[],
        ProtocolError::InvalidAssociatedTokenAccount,
    );
}

#[test]
fn wrong_escrow_authority_and_bond_vault() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(4);
    // Create ATA for a different wallet and try to use as escrow
    let other = Keypair::new();
    let other_ata = env.create_ata(&other.pubkey());
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    env.mint_to(&buyer_ata, JOB_AMOUNT * 2);
    let fund = env.ix_fund_job(&job, buyer_ata, other_ata, TOKEN_PROGRAM_ID);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(
        &buyer,
        &[fund],
        &[],
        ProtocolError::InvalidAssociatedTokenAccount,
    );

    let mut env = setup();
    env.initialize_config();
    env.register_provider();
    let ata = env.create_ata(&env.provider.pubkey());
    env.mint_to(&ata, MIN_BOND);
    let wrong_vault = env.create_ata(&env.buyer.pubkey());
    let dep = env.ix_deposit_bond(MIN_BOND, ata, wrong_vault);
    let provider = env.provider.insecure_clone();
    env.send_err_code(
        &provider,
        &[dep],
        &[],
        ProtocolError::InvalidAssociatedTokenAccount,
    );
}

#[test]
fn frozen_delegated_on_fund_and_accept_work() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(5);
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    env.mint_to(&buyer_ata, JOB_AMOUNT * 2);
    let escrow = env.ensure_escrow(&job);
    env.force_frozen(&buyer_ata);
    let fund = env.ix_fund_job(&job, buyer_ata, escrow, TOKEN_PROGRAM_ID);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(&buyer, &[fund], &[], ProtocolError::TokenAccountFrozen);

    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(6);
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    env.mint_to(&buyer_ata, JOB_AMOUNT * 2);
    let escrow = env.ensure_escrow(&job);
    env.force_delegate(&buyer_ata, &Pubkey::new_unique(), 1);
    let fund = env.ix_fund_job(&job, buyer_ata, escrow, TOKEN_PROGRAM_ID);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(
        &buyer,
        &[fund],
        &[],
        ProtocolError::InvalidTokenAccountAuthority,
    );
}

#[test]
fn escrow_below_principal_and_surplus_handling() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(7);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 7);
    env.submit_receipt(&job, &receipt);
    // Drain escrow below principal via direct patch
    let escrow = env.escrow_ata(&job);
    let mut acc = env.svm.get_account(&escrow).expect("escrow");
    acc.data[64..72].copy_from_slice(&1u64.to_le_bytes());
    env.svm.set_account(escrow, acc).expect("set");
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(
        &buyer,
        &[env.ix_accept_work(&job)],
        &[],
        ProtocolError::EscrowUnexpectedBalance,
    );

    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(8);
    env.fund_job(&job);
    env.accept_job(&job);
    let escrow = env.escrow_ata(&job);
    env.mint_to(&escrow, 321);
    let receipt = env.make_receipt(&job, 8);
    env.submit_receipt(&job, &receipt);
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    let provider_ata = env.create_ata(&env.provider.pubkey());
    let buyer_before = env.token_balance(&buyer_ata);
    let provider_before = env.token_balance(&provider_ata);
    env.accept_work(&job);
    assert_eq!(
        env.token_balance(&provider_ata),
        provider_before + JOB_AMOUNT
    );
    assert_eq!(env.token_balance(&buyer_ata), buyer_before + 321);
    assert_eq!(env.token_balance(&escrow), 0);
}

#[test]
fn principal_not_changed_by_donation() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(9);
    env.fund_job(&job);
    let escrow = env.escrow_ata(&job);
    env.mint_to(&escrow, 50);
    assert_eq!(env.read_job(&job).amount, JOB_AMOUNT);
    assert_eq!(env.token_balance(&escrow), JOB_AMOUNT + 50);
}

#[test]
fn wrong_provider_token_on_accept_work() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(10);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 10);
    env.submit_receipt(&job, &receipt);
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    let wrong = env.create_ata(&env.admin.pubkey());
    let ix = Instruction {
        program_id: env.program_id,
        accounts: vec![
            AccountMeta::new_readonly(env.buyer.pubkey(), true),
            AccountMeta::new(job, false),
            AccountMeta::new(env.bond_pda(), false),
            AccountMeta::new(env.escrow_ata(&job), false),
            AccountMeta::new(wrong, false),
            AccountMeta::new(buyer_ata, false),
            AccountMeta::new_readonly(env.mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data: encode_empty(InstructionKind::AcceptWork)
            .expect("e")
            .to_vec(),
    };
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(
        &buyer,
        &[ix],
        &[],
        ProtocolError::InvalidAssociatedTokenAccount,
    );
    let _ = get_associated_token_address;
}
