mod common;

use agentbond_types::{
    encode_empty, encode_initialize_config, encode_set_paused, CreateJobPayload, InstructionKind,
    ProtocolError,
};
use common::{
    address_bytes, setup, token_2022_id, DECIMALS, GENESIS, JOB_AMOUNT, MIN_BOND, START_TS,
};
use ed25519_dalek::{Keypair as DalekKeypair, PublicKey, SecretKey};
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use spl_token::ID as TOKEN_PROGRAM_ID;

#[test]
fn config_initializes_correctly() {
    let mut env = setup();
    env.initialize_config();
    let cfg = env.read_config();
    assert!(!cfg.paused);
    assert_eq!(cfg.admin, address_bytes(&env.admin.pubkey()));
    assert_eq!(cfg.genesis_hash, GENESIS);
    assert_eq!(cfg.allowed_mint, address_bytes(&env.mint));
    assert_eq!(cfg.token_program, TOKEN_PROGRAM_ID.to_bytes());
    assert_eq!(cfg.mint_decimals, DECIMALS);
    assert_eq!(cfg.min_provider_bond, MIN_BOND);
    assert_eq!(cfg.challenge_duration_seconds, 3_600);
}

#[test]
fn config_wrong_pda_rejected() {
    let mut env = setup();
    let payload = env.default_config_payload();
    let ix = Instruction {
        program_id: env.program_id,
        accounts: vec![
            AccountMeta::new(env.admin.pubkey(), true),
            AccountMeta::new(Pubkey::new_unique(), false),
            AccountMeta::new_readonly(Pubkey::default(), false),
        ],
        data: encode_initialize_config(&payload).to_vec(),
    };
    let admin = env.admin.insecure_clone();
    env.send_err_code(&admin, &[ix], &[], ProtocolError::InvalidPda);
}

#[test]
fn config_reinitialize_rejected() {
    let mut env = setup();
    env.initialize_config();
    env.svm.expire_blockhash();
    let mut payload = env.default_config_payload();
    payload.min_provider_bond = MIN_BOND + 1;
    let ix = env.ix_initialize_config(&payload);
    let admin = env.admin.insecure_clone();
    env.send_err_code(&admin, &[ix], &[], ProtocolError::AlreadyInitialized);
}

#[test]
fn config_wrong_admin_pause_rejected() {
    let mut env = setup();
    env.initialize_config();
    let stranger = Keypair::new();
    env.svm
        .airdrop(&stranger.pubkey(), 10_000_000_000)
        .expect("airdrop");
    let ix = Instruction {
        program_id: env.program_id,
        accounts: vec![
            AccountMeta::new_readonly(stranger.pubkey(), true),
            AccountMeta::new(env.config_pda(), false),
        ],
        data: encode_set_paused(true).to_vec(),
    };
    env.send_err_code(&stranger, &[ix], &[], ProtocolError::Unauthorized);
}

#[test]
fn config_pause_and_unpause() {
    let mut env = setup();
    env.initialize_config();
    env.set_paused(true);
    assert!(env.read_config().paused);
    env.set_paused(false);
    assert!(!env.read_config().paused);
}

#[test]
fn config_invalid_challenge_duration_rejected() {
    let mut env = setup();
    let mut payload = env.default_config_payload();
    payload.challenge_duration_seconds = 0;
    let ix = env.ix_initialize_config(&payload);
    let admin = env.admin.insecure_clone();
    env.send_err_code(&admin, &[ix], &[], ProtocolError::InvalidConfig);
}

#[test]
fn config_zero_min_bond_rejected() {
    let mut env = setup();
    let mut payload = env.default_config_payload();
    payload.min_provider_bond = 0;
    let ix = env.ix_initialize_config(&payload);
    let admin = env.admin.insecure_clone();
    env.send_err_code(&admin, &[ix], &[], ProtocolError::InvalidAmount);
}

#[test]
fn config_non_legacy_token_program_rejected() {
    let mut env = setup();
    let mut payload = env.default_config_payload();
    payload.token_program = token_2022_id().to_bytes();
    let ix = env.ix_initialize_config(&payload);
    let admin = env.admin.insecure_clone();
    env.send_err_code(&admin, &[ix], &[], ProtocolError::InvalidTokenProgram);
}

#[test]
fn config_mint_decimals_mismatch_rejected_on_fund() {
    // InitializeConfig stores claimed decimals; FundJob validates against the mint account.
    // DepositBond also validates decimals, so skip bonding for this check.
    let mut env = setup();
    let mut payload = env.default_config_payload();
    payload.mint_decimals = DECIMALS + 1;
    let ix = env.ix_initialize_config(&payload);
    let admin = env.admin.insecure_clone();
    env.send_ok(&admin, &[ix], &[]);
    env.register_provider();
    let job = env.create_job(1);
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    env.mint_to(&buyer_ata, JOB_AMOUNT * 2);
    let escrow = env.ensure_escrow(&job);
    let fund = env.ix_fund_job(&job, buyer_ata, escrow, TOKEN_PROGRAM_ID);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(&buyer, &[fund], &[], ProtocolError::InvalidMint);
}

#[test]
fn pause_blocks_entry_paths() {
    let mut env = setup();
    env.bootstrap_ready();
    env.set_paused(true);

    let provider = env.provider.insecure_clone();
    let stranger = Keypair::new();
    env.svm
        .airdrop(&stranger.pubkey(), 10_000_000_000)
        .expect("airdrop");
    // Register (new authority) blocked
    let ix = Instruction {
        program_id: env.program_id,
        accounts: vec![
            AccountMeta::new(stranger.pubkey(), true),
            AccountMeta::new_readonly(env.config_pda(), false),
            AccountMeta::new(env.provider_pda_for(&stranger.pubkey()), false),
            AccountMeta::new_readonly(Pubkey::default(), false),
        ],
        data: encode_empty(InstructionKind::RegisterProvider)
            .expect("empty")
            .to_vec(),
    };
    env.send_err_code(&stranger, &[ix], &[], ProtocolError::ProtocolPaused);

    // Deposit blocked
    let ata = env.create_ata(&env.provider.pubkey());
    env.mint_to(&ata, MIN_BOND);
    let vault = env.ensure_bond_vault();
    let dep = env.ix_deposit_bond(MIN_BOND, ata, vault);
    env.send_err_code(&provider, &[dep], &[], ProtocolError::ProtocolPaused);

    // Create blocked
    let payload = env.create_job_payload(99);
    let create = env.ix_create_job(&payload);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(&buyer, &[create], &[], ProtocolError::ProtocolPaused);

    // Fund blocked
    env.set_paused(false);
    let job = env.create_job(2);
    env.set_paused(true);
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    env.mint_to(&buyer_ata, JOB_AMOUNT * 2);
    let escrow = env.ensure_escrow(&job);
    let fund = env.ix_fund_job(&job, buyer_ata, escrow, TOKEN_PROGRAM_ID);
    env.send_err_code(&buyer, &[fund], &[], ProtocolError::ProtocolPaused);

    // Accept blocked
    env.set_paused(false);
    env.fund_job(&job);
    env.set_paused(true);
    let accept = env.ix_accept_job(&job);
    env.send_err_code(&provider, &[accept], &[], ProtocolError::ProtocolPaused);
}

#[test]
fn pause_does_not_trap_funds() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(3);
    env.fund_job(&job);
    env.accept_job(&job);
    env.set_paused(true);

    let receipt = env.make_receipt(&job, 3);
    env.submit_receipt(&job, &receipt);
    env.accept_work(&job);
    env.assert_job_state(&job, agentbond_types::JobState::Settled);
    env.close_job(&job);
    env.assert_account_closed(&job);

    // Withdraw unlocked bond while paused
    let before = env.read_bond().deposited;
    assert!(before > 0);
    env.withdraw_bond(before);
    assert_eq!(env.read_bond().deposited, 0);
}

#[test]
fn pause_allows_challenge_settle_refund() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(4);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 4);
    env.submit_receipt(&job, &receipt);
    env.set_paused(true);
    env.challenge_work(&job);
    env.set_clock(START_TS + 10 + 3_600);
    env.resolve_timeout_settle(&job, true);
    env.assert_job_state(&job, agentbond_types::JobState::Settled);

    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(5);
    env.fund_job(&job);
    env.set_paused(true);
    env.set_clock(START_TS + 200);
    env.resolve_timeout_refund(&job);
    env.assert_job_state(&job, agentbond_types::JobState::Refunded);
}

#[test]
fn config_writable_rejected_on_common_ops() {
    let mut env = setup();
    env.bootstrap_ready();
    // Create job with writable config must fail.
    let payload = env.create_job_payload(7);
    let mut create = env.ix_create_job(&payload);
    create.accounts[1] = AccountMeta::new(env.config_pda(), false);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(&buyer, &[create], &[], ProtocolError::InvalidConfig);

    let job = env.create_job(8);
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    env.mint_to(&buyer_ata, JOB_AMOUNT * 2);
    let escrow = env.ensure_escrow(&job);
    let mut fund = env.ix_fund_job(&job, buyer_ata, escrow, TOKEN_PROGRAM_ID);
    fund.accounts[1] = AccountMeta::new(env.config_pda(), false);
    env.send_err_code(&buyer, &[fund], &[], ProtocolError::InvalidConfig);
}

#[test]
fn provider_register_and_duplicate() {
    let mut env = setup();
    env.initialize_config();
    env.register_provider();
    let provider = env.read_provider();
    assert_eq!(provider.authority, address_bytes(&env.provider.pubkey()));
    let ix = env.ix_register_provider();
    let payer = env.provider.insecure_clone();
    env.send_err_code(&payer, &[ix], &[], ProtocolError::AlreadyInitialized);
}

#[test]
fn provider_wrong_pda_rejected() {
    let mut env = setup();
    env.initialize_config();
    let ix = Instruction {
        program_id: env.program_id,
        accounts: vec![
            AccountMeta::new(env.provider.pubkey(), true),
            AccountMeta::new_readonly(env.config_pda(), false),
            AccountMeta::new(Pubkey::new_unique(), false),
            AccountMeta::new_readonly(Pubkey::default(), false),
        ],
        data: encode_empty(InstructionKind::RegisterProvider)
            .expect("empty")
            .to_vec(),
    };
    let payer = env.provider.insecure_clone();
    env.send_err_code(&payer, &[ix], &[], ProtocolError::InvalidPda);
}

#[test]
fn provider_wrong_authority_for_keys() {
    let mut env = setup();
    env.initialize_config();
    env.register_provider();
    let stranger = Keypair::new();
    env.svm
        .airdrop(&stranger.pubkey(), 10_000_000_000)
        .expect("airdrop");
    let key = [1u8; 32];
    let ix = Instruction {
        program_id: env.program_id,
        accounts: vec![
            AccountMeta::new_readonly(stranger.pubkey(), true),
            AccountMeta::new(env.provider_pda(), false),
        ],
        data: agentbond_types::encode_add_execution_key(&key).to_vec(),
    };
    env.send_err_code(&stranger, &[ix], &[], ProtocolError::Unauthorized);
}

#[test]
fn execution_keys_add_duplicate_zero_max_revoke_compact() {
    let mut env = setup();
    env.initialize_config();
    env.register_provider();

    let zero = [0u8; 32];
    let payer = env.provider.insecure_clone();
    env.send_err_code(
        &payer,
        &[env.ix_add_key(&zero)],
        &[],
        ProtocolError::InvalidPubkey,
    );

    let keys: Vec<[u8; 32]> = (1u8..=5)
        .map(|i| {
            let secret = SecretKey::from_bytes(&[i; 32]).expect("secret");
            PublicKey::from(&secret).to_bytes()
        })
        .collect();

    for key in &keys[..4] {
        env.add_execution_key_bytes(key);
    }
    assert_eq!(env.read_provider().execution_key_count, 4);
    env.send_err_code(
        &payer,
        &[env.ix_add_key(&keys[0])],
        &[],
        ProtocolError::DuplicateExecutionKey,
    );
    env.send_err_code(
        &payer,
        &[env.ix_add_key(&keys[4])],
        &[],
        ProtocolError::ExecutionKeyLimit,
    );

    // Revoke second key and ensure compaction (count decreases; key absent).
    env.revoke_execution_key(&keys[1]);
    let provider = env.read_provider();
    assert_eq!(provider.execution_key_count, 3);
    assert!(!provider.contains_execution_key(&keys[1]));
    env.send_err_code(
        &payer,
        &[env.ix_revoke_key(&keys[1])],
        &[],
        ProtocolError::ExecutionKeyNotFound,
    );

    // Revoked key cannot submit receipt
    env.add_execution_key(); // env.exec
    env.deposit_bond(MIN_BOND * 2);
    let job = env.create_job(10);
    env.fund_job(&job);
    env.accept_job(&job);
    let revoked = DalekKeypair {
        secret: SecretKey::from_bytes(&[2u8; 32]).expect("s"),
        public: PublicKey::from(&SecretKey::from_bytes(&[2u8; 32]).expect("s2")),
    };
    // Ensure key 2 was the revoked one from keys[1]
    let receipt = env.make_receipt(&job, 10);
    let ixs = env.submit_receipt_ixs(&job, &receipt, &revoked);
    env.send_err_code(&payer, &ixs, &[], ProtocolError::InvalidSignature);
}

#[test]
fn inactive_provider_rejected_on_accept() {
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(11);
    env.fund_job(&job);
    env.set_provider_inactive();
    let accept = env.ix_accept_job(&job);
    let payer = env.provider.insecure_clone();
    env.send_err_code(&payer, &[accept], &[], ProtocolError::ProviderInactive);
}

#[test]
fn create_job_rejects_inactive_provider() {
    let mut env = setup();
    env.bootstrap_ready();
    env.set_provider_inactive();
    let payload = env.create_job_payload(12);
    let ix = env.ix_create_job(&payload);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(&buyer, &[ix], &[], ProtocolError::ProviderInactive);
}

#[test]
fn config_wrong_owner_rejected_on_ops() {
    let mut env = setup();
    env.bootstrap_ready();
    let config = env.config_pda();
    let mut acc = env.svm.get_account(&config).expect("config");
    acc.owner = Pubkey::new_unique();
    env.svm.set_account(config, acc).expect("set");
    let payload = CreateJobPayload {
        job_nonce: 13,
        amount: JOB_AMOUNT,
        request_hash: [9u8; 32],
        fund_deadline: START_TS + 100,
        accept_deadline: START_TS + 200,
        work_deadline: START_TS + 300,
        auto_settle_deadline: START_TS + 400,
    };
    let ix = env.ix_create_job(&payload);
    let buyer = env.buyer.insecure_clone();
    env.send_err_code(&buyer, &[ix], &[], ProtocolError::InvalidOwner);
}
