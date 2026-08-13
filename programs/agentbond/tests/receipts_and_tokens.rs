mod common;

use agentbond_types::{encode_empty, encode_submit_receipt, InstructionKind, ProtocolError};
use common::{setup, START_TS};
use ed25519_dalek::{Keypair as DalekKeypair, PublicKey, SecretKey};
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use spl_associated_token_account_client::address::get_associated_token_address;
use spl_token::ID as TOKEN_PROGRAM_ID;

#[test]
fn receipt_wrong_program_id_rejected() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(10);
    env.fund_job(&job);
    env.accept_job(&job);
    let mut receipt = env.make_receipt(&job, 10);
    receipt.program_id = [0xab; 32];
    let ixs = env.submit_receipt_ixs(&job, &receipt, &env.exec);
    let payer = env.provider.insecure_clone();
    env.send_err_code(&payer, &ixs, &[], ProtocolError::InvalidReceiptField);
}

#[test]
fn receipt_wrong_genesis_rejected() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(11);
    env.fund_job(&job);
    env.accept_job(&job);
    let mut receipt = env.make_receipt(&job, 11);
    receipt.genesis_hash = [0xcd; 32];
    let ixs = env.submit_receipt_ixs(&job, &receipt, &env.exec);
    let payer = env.provider.insecure_clone();
    env.send_err_code(&payer, &ixs, &[], ProtocolError::InvalidReceiptField);
}

#[test]
fn receipt_missing_ed25519_rejected() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(12);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 12);
    let config = env.config_pda();
    let provider_pda = env.provider_pda();
    let ix = Instruction {
        program_id: env.program_id,
        accounts: vec![
            AccountMeta::new_readonly(config, false),
            AccountMeta::new_readonly(provider_pda, false),
            AccountMeta::new(job, false),
            AccountMeta::new_readonly(
                Pubkey::from_str_const("Sysvar1nstructions1111111111111111111111111"),
                false,
            ),
        ],
        data: encode_submit_receipt(&receipt).expect("encode").to_vec(),
    };
    let payer = env.provider.insecure_clone();
    env.send_err_code(&payer, &[ix], &[], ProtocolError::MissingEd25519Instruction);
}

#[test]
fn receipt_wrong_signer_key_rejected() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(13);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 13);
    let secret = SecretKey::from_bytes(&[9u8; 32]).expect("secret");
    let public = PublicKey::from(&secret);
    let wrong = DalekKeypair { secret, public };
    let ixs = env.submit_receipt_ixs(&job, &receipt, &wrong);
    let payer = env.provider.insecure_clone();
    env.send_err_code(&payer, &ixs, &[], ProtocolError::InvalidSignature);
}

#[test]
fn receipt_expired_rejected() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(14);
    env.fund_job(&job);
    env.accept_job(&job);
    let mut receipt = env.make_receipt(&job, 14);
    receipt.expires_at = START_TS - 1;
    receipt.created_at = START_TS - 10;
    let ixs = env.submit_receipt_ixs(&job, &receipt, &env.exec);
    let payer = env.provider.insecure_clone();
    env.send_err_code(&payer, &ixs, &[], ProtocolError::ReceiptExpired);
}

#[test]
fn wrong_token_program_rejected_on_fund() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(15);
    let config = env.config_pda();
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    env.mint_to(&buyer_ata, 10_000);
    let escrow = get_associated_token_address(&job, &env.mint);
    let create_escrow =
        spl_associated_token_account_client::instruction::create_associated_token_account(
            &env.buyer.pubkey(),
            &job,
            &env.mint,
            &TOKEN_PROGRAM_ID,
        );
    let buyer = env.buyer.insecure_clone();
    env.send_ok(&buyer, &[create_escrow], &[]);
    let token_2022 = Pubkey::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
    let ix = Instruction {
        program_id: env.program_id,
        accounts: vec![
            AccountMeta::new_readonly(buyer.pubkey(), true),
            AccountMeta::new_readonly(config, false),
            AccountMeta::new(job, false),
            AccountMeta::new(buyer_ata, false),
            AccountMeta::new(escrow, false),
            AccountMeta::new_readonly(env.mint, false),
            AccountMeta::new_readonly(token_2022, false),
        ],
        data: encode_empty(InstructionKind::FundJob)
            .expect("empty")
            .to_vec(),
    };
    env.send_err_code(&buyer, &[ix], &[], ProtocolError::InvalidTokenProgram);
}

#[test]
fn reinitialize_config_rejected() {
    let mut env = setup();
    env.initialize_config();
    env.svm.expire_blockhash();
    let config = env.config_pda();
    use agentbond_types::{encode_initialize_config, InitializeConfigPayload};
    use common::{DECIMALS, GENESIS, MIN_BOND};
    let payload = InitializeConfigPayload {
        genesis_hash: GENESIS,
        allowed_mint: env.mint.to_bytes(),
        token_program: TOKEN_PROGRAM_ID.to_bytes(),
        mint_decimals: DECIMALS,
        min_provider_bond: MIN_BOND + 1,
        challenge_duration_seconds: 3_600,
    };
    let ix = Instruction {
        program_id: env.program_id,
        accounts: vec![
            AccountMeta::new(env.admin.pubkey(), true),
            AccountMeta::new(config, false),
            AccountMeta::new_readonly(Pubkey::default(), false),
        ],
        data: encode_initialize_config(&payload).to_vec(),
    };
    let admin = env.admin.insecure_clone();
    env.send_err_code(&admin, &[ix], &[], ProtocolError::AlreadyInitialized);
}

#[test]
fn fund_cu_logged() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(16);
    let config = env.config_pda();
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    env.mint_to(&buyer_ata, 20_000);
    let escrow = get_associated_token_address(&job, &env.mint);
    let create_escrow =
        spl_associated_token_account_client::instruction::create_associated_token_account(
            &env.buyer.pubkey(),
            &job,
            &env.mint,
            &TOKEN_PROGRAM_ID,
        );
    let buyer = env.buyer.insecure_clone();
    env.send_ok(&buyer, &[create_escrow], &[]);
    let ix = Instruction {
        program_id: env.program_id,
        accounts: vec![
            AccountMeta::new_readonly(buyer.pubkey(), true),
            AccountMeta::new_readonly(config, false),
            AccountMeta::new(job, false),
            AccountMeta::new(buyer_ata, false),
            AccountMeta::new(escrow, false),
            AccountMeta::new_readonly(env.mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data: encode_empty(InstructionKind::FundJob)
            .expect("empty")
            .to_vec(),
    };
    let meta = env
        .send(
            &buyer,
            &[
                solana_compute_budget_interface::ComputeBudgetInstruction::set_compute_unit_limit(
                    1_400_000,
                ),
                ix,
            ],
            &[],
        )
        .expect("fund");
    println!("CU fund_job={}", meta.compute_units_consumed);
    assert!(meta.compute_units_consumed > 0);
}
