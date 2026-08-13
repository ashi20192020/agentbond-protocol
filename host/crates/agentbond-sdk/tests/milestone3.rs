//! Milestone 3 SDK coverage: PDAs, ATAs, decode rejection, builders, receipts, plans.

use agentbond_sdk::{
    AccountMetaPlan, InstructionPlan, PROGRAM_ID_BYTES, bond_vault_ata,
    build_ed25519_verify_instruction, build_submit_receipt_plan, challenge_pda, config_pda,
    decode_challenge, decode_config, decode_job, decode_provider, decode_provider_bond,
    job_escrow_ata, job_pda, plan_accept_job, plan_accept_work, plan_add_execution_key,
    plan_challenge_work, plan_close_job, plan_create_job, plan_deposit_bond,
    plan_expire_unaccepted, plan_expire_unfunded, plan_fund_job, plan_initialize_config,
    plan_register_provider, plan_resolve_timeout_refund, plan_resolve_timeout_settle,
    plan_revoke_execution_key, plan_set_paused, plan_slash_bond, plan_withdraw_bond, program_id,
    provider_bond_pda, provider_pda, receipt_digest, user_settlement_ata, validate_receipt,
};
use agentbond_types::{
    AgentBondWorkReceiptV1, CHALLENGE_ACCOUNT_LEN, CONFIG_ACCOUNT_LEN, ChallengeAccount,
    ConfigAccount, CreateJobPayload, InitializeConfigPayload, InstructionKind, JOB_ACCOUNT_LEN,
    JobAccount, JobState, PROVIDER_ACCOUNT_DISCRIMINATOR, PROVIDER_ACCOUNT_LEN,
    PROVIDER_BOND_ACCOUNT_LEN, PROVIDER_STATUS_ACTIVE, ProviderAccount, ProviderBondAccount,
    RECEIPT_ENCODED_LEN, bond_seed_parts, challenge_seed_parts, config_seed_parts,
    encode_add_execution_key, encode_challenge_work, encode_create_job, encode_deposit_bond,
    encode_empty, encode_initialize_config, encode_revoke_execution_key, encode_set_paused,
    encode_submit_receipt, encode_withdraw_bond, job_nonce_le_bytes, job_seed_parts,
    provider_seed_parts,
};
use solana_instruction::AccountMeta;
use solana_pubkey::Pubkey;
use spl_associated_token_account_client::address::get_associated_token_address_with_program_id;
use spl_token::ID as TOKEN_PROGRAM_ID;

fn pk(bytes: [u8; 32]) -> Pubkey {
    Pubkey::new_from_array(bytes)
}

fn sample_receipt() -> AgentBondWorkReceiptV1 {
    AgentBondWorkReceiptV1 {
        program_id: PROGRAM_ID_BYTES,
        genesis_hash: [7u8; 32],
        job: [1u8; 32],
        buyer: [2u8; 32],
        provider: [3u8; 32],
        request_hash: [9u8; 32],
        result_hash: [4u8; 32],
        artifact_hash: [5u8; 32],
        software_hash: [6u8; 32],
        job_nonce: 1,
        created_at: 1_700_000_000,
        expires_at: 1_700_000_400,
    }
}

fn assert_account(meta: &AccountMeta, expected: &Pubkey, writable: bool, signer: bool) {
    assert_eq!(&meta.pubkey, expected, "account pubkey mismatch");
    assert_eq!(meta.is_writable, writable, "writable flag for {expected}");
    assert_eq!(meta.is_signer, signer, "signer flag for {expected}");
}

fn assert_plan_account(
    plan_meta: &AccountMetaPlan,
    expected: &Pubkey,
    writable: bool,
    signer: bool,
) {
    assert_eq!(plan_meta.pubkey, expected.to_string());
    assert_eq!(plan_meta.is_writable, writable);
    assert_eq!(plan_meta.is_signer, signer);
}

#[test]
fn program_id_bytes_match_placeholder() {
    assert_eq!(program_id().to_bytes(), PROGRAM_ID_BYTES);
    assert_eq!(
        PROGRAM_ID_BYTES,
        [
            0x0a, 0x9e, 0xb1, 0x6d, 0x2c, 0x84, 0x3f, 0x51, 0x7a, 0xc2, 0x08, 0xd4, 0x6e, 0x35,
            0x91, 0xbf, 0x14, 0x67, 0xda, 0x2c, 0x58, 0x03, 0xee, 0x49, 0xb7, 0x1f, 0x85, 0x20,
            0xcd, 0x63, 0xa4, 0x7e,
        ]
    );
}

#[test]
fn pda_parity_with_types_seed_helpers() {
    let program = program_id();
    let authority = pk([1u8; 32]);
    let mint = pk([2u8; 32]);
    let buyer = pk([3u8; 32]);
    let provider = pk([4u8; 32]);
    let job_key = pk([5u8; 32]);
    let nonce = 9u64;

    let (cfg_addr, cfg_bump) = Pubkey::find_program_address(&config_seed_parts(), &program);
    let sdk_cfg = config_pda(&program).expect("config pda");
    assert_eq!(sdk_cfg.address, cfg_addr);
    assert_eq!(sdk_cfg.bump, cfg_bump);

    let auth_b = authority.to_bytes();
    let (prov_addr, prov_bump) =
        Pubkey::find_program_address(&provider_seed_parts(&auth_b), &program);
    let sdk_prov = provider_pda(&program, &authority).expect("provider pda");
    assert_eq!(sdk_prov.address, prov_addr);
    assert_eq!(sdk_prov.bump, prov_bump);

    let mint_b = mint.to_bytes();
    let (bond_addr, bond_bump) =
        Pubkey::find_program_address(&bond_seed_parts(&auth_b, &mint_b), &program);
    let sdk_bond = provider_bond_pda(&program, &authority, &mint).expect("bond pda");
    assert_eq!(sdk_bond.address, bond_addr);
    assert_eq!(sdk_bond.bump, bond_bump);

    let buyer_b = buyer.to_bytes();
    let provider_b = provider.to_bytes();
    let nonce_b = job_nonce_le_bytes(nonce);
    let (job_addr, job_bump) =
        Pubkey::find_program_address(&job_seed_parts(&buyer_b, &provider_b, &nonce_b), &program);
    let sdk_job = job_pda(&program, &buyer, &provider, nonce).expect("job pda");
    assert_eq!(sdk_job.address, job_addr);
    assert_eq!(sdk_job.bump, job_bump);

    let job_b = job_key.to_bytes();
    let (chal_addr, chal_bump) =
        Pubkey::find_program_address(&challenge_seed_parts(&job_b), &program);
    let sdk_chal = challenge_pda(&program, &job_key).expect("challenge pda");
    assert_eq!(sdk_chal.address, chal_addr);
    assert_eq!(sdk_chal.bump, chal_bump);
}

#[test]
fn pda_deterministic_for_fixed_inputs() {
    let program = program_id();
    let authority = pk([11u8; 32]);
    let mint = pk([22u8; 32]);
    let buyer = pk([33u8; 32]);
    let provider = pk([44u8; 32]);

    let a = config_pda(&program).expect("config").address;
    let b = config_pda(&program).expect("config").address;
    assert_eq!(a, b);

    let p1 = provider_pda(&program, &authority)
        .expect("provider")
        .address;
    let p2 = provider_pda(&program, &authority)
        .expect("provider")
        .address;
    assert_eq!(p1, p2);

    let bond = provider_bond_pda(&program, &authority, &mint)
        .expect("bond")
        .address;
    let job = job_pda(&program, &buyer, &provider, 42)
        .expect("job")
        .address;
    let challenge = challenge_pda(&program, &job).expect("challenge").address;

    // Distinct seeds produce distinct addresses.
    assert_ne!(a, p1);
    assert_ne!(p1, bond);
    assert_ne!(bond, job);
    assert_ne!(job, challenge);

    let job_other = job_pda(&program, &buyer, &provider, 43)
        .expect("job other")
        .address;
    assert_ne!(job, job_other);
}

#[test]
fn ata_derivation_matches_spl_helper() {
    let owner = pk([9u8; 32]);
    let mint = pk([8u8; 32]);
    let bond = pk([7u8; 32]);
    let job = pk([6u8; 32]);

    let expected_user =
        get_associated_token_address_with_program_id(&owner, &mint, &TOKEN_PROGRAM_ID);
    assert_eq!(user_settlement_ata(&owner, &mint), expected_user);

    let expected_bond =
        get_associated_token_address_with_program_id(&bond, &mint, &TOKEN_PROGRAM_ID);
    assert_eq!(bond_vault_ata(&bond, &mint), expected_bond);

    let expected_escrow =
        get_associated_token_address_with_program_id(&job, &mint, &TOKEN_PROGRAM_ID);
    assert_eq!(job_escrow_ata(&job, &mint), expected_escrow);
}

#[test]
fn decode_config_accepts_valid_and_rejects_bad_inputs() {
    let program = program_id();
    let addr = config_pda(&program).expect("config").address;
    let cfg = ConfigAccount {
        bump: 255,
        paused: false,
        admin: [9u8; 32],
        genesis_hash: [1u8; 32],
        allowed_mint: [2u8; 32],
        token_program: TOKEN_PROGRAM_ID.to_bytes(),
        mint_decimals: 6,
        min_provider_bond: 1_000,
        challenge_duration_seconds: 60,
    };
    let data = cfg.encode();
    let decoded = decode_config(&program, &addr, &program, &data).expect("decode config");
    assert_eq!(decoded, cfg);

    assert!(matches!(
        decode_config(&program, &addr, &pk([1u8; 32]), &data),
        Err(agentbond_sdk::SdkError::WrongOwner)
    ));
    assert!(matches!(
        decode_config(&program, &pk([1u8; 32]), &program, &data),
        Err(agentbond_sdk::SdkError::WrongAddress)
    ));
    let mut short = data.to_vec();
    short.pop();
    assert!(decode_config(&program, &addr, &program, &short).is_err());
    let mut bad_disc = data;
    bad_disc[0] = 0xFF;
    assert!(decode_config(&program, &addr, &program, &bad_disc).is_err());
}

#[test]
fn decode_provider_job_bond_challenge_rejection() {
    let program = program_id();
    let authority = pk([10u8; 32]);
    let mint = pk([20u8; 32]);
    let buyer = pk([30u8; 32]);
    let provider_auth = authority;

    let provider_addr = provider_pda(&program, &authority)
        .expect("provider")
        .address;
    let provider = ProviderAccount {
        bump: 1,
        status: PROVIDER_STATUS_ACTIVE,
        authority: authority.to_bytes(),
        execution_key_count: 0,
        execution_keys: [[0u8; 32]; 4],
    };
    let pdata = provider.encode().expect("encode provider");
    assert_eq!(
        decode_provider(&program, &provider_addr, &program, &pdata).expect("ok"),
        provider
    );
    assert!(matches!(
        decode_provider(&program, &pk([99u8; 32]), &program, &pdata),
        Err(agentbond_sdk::SdkError::WrongAddress)
    ));
    let mut bad = pdata;
    bad[0] = PROVIDER_ACCOUNT_DISCRIMINATOR.wrapping_add(10);
    assert!(decode_provider(&program, &provider_addr, &program, &bad).is_err());

    let bond_addr = provider_bond_pda(&program, &authority, &mint)
        .expect("bond")
        .address;
    let bond = ProviderBondAccount {
        bump: 2,
        provider: authority.to_bytes(),
        mint: mint.to_bytes(),
        token_program: TOKEN_PROGRAM_ID.to_bytes(),
        deposited: 100,
        locked: 10,
    };
    let bdata = bond.encode().expect("encode bond");
    assert_eq!(
        decode_provider_bond(&program, &bond_addr, &program, &bdata).expect("ok"),
        bond
    );
    assert!(matches!(
        decode_provider_bond(&program, &bond_addr, &pk([1u8; 32]), &bdata),
        Err(agentbond_sdk::SdkError::WrongOwner)
    ));
    let mut short = bdata.to_vec();
    short.truncate(PROVIDER_BOND_ACCOUNT_LEN - 1);
    assert!(decode_provider_bond(&program, &bond_addr, &program, &short).is_err());

    let job_addr = job_pda(&program, &buyer, &provider_auth, 7)
        .expect("job")
        .address;
    let job = JobAccount {
        bump: 3,
        state: JobState::Funded,
        buyer: buyer.to_bytes(),
        provider: provider_auth.to_bytes(),
        mint: mint.to_bytes(),
        token_program: TOKEN_PROGRAM_ID.to_bytes(),
        amount: 50,
        job_nonce: 7,
        fund_deadline: 100,
        accept_deadline: 200,
        work_deadline: 300,
        auto_settle_deadline: 400,
        receipt_digest: [0u8; 32],
        request_hash: [9u8; 32],
        locked_bond: 0,
        mint_decimals: 6,
    };
    let jdata = job.encode();
    assert_eq!(
        decode_job(&program, &job_addr, &program, &jdata).expect("ok"),
        job
    );
    let mut wrong_layout = jdata;
    wrong_layout[3] = 255; // invalid job state
    assert!(decode_job(&program, &job_addr, &program, &wrong_layout).is_err());
    let mut wrong_len = jdata.to_vec();
    wrong_len.push(0);
    assert!(decode_job(&program, &job_addr, &program, &wrong_len).is_err());
    assert_eq!(jdata.len(), JOB_ACCOUNT_LEN);

    let challenge_addr = challenge_pda(&program, &job_addr)
        .expect("challenge")
        .address;
    let challenge = ChallengeAccount {
        bump: 4,
        status: ChallengeAccount::STATUS_OPEN,
        job: job_addr.to_bytes(),
        buyer: buyer.to_bytes(),
        reason_hash: [5u8; 32],
        bond_amount: 0,
        deadline: 500,
    };
    let cdata = challenge.encode().expect("encode challenge");
    assert_eq!(cdata.len(), CHALLENGE_ACCOUNT_LEN);
    assert_eq!(
        decode_challenge(&program, &challenge_addr, &program, &cdata).expect("ok"),
        challenge
    );
    assert!(matches!(
        decode_challenge(&program, &pk([1u8; 32]), &program, &cdata),
        Err(agentbond_sdk::SdkError::WrongAddress)
    ));
}

#[test]
fn instruction_builders_account_order_flags_and_payloads() {
    let program = program_id();
    let admin = pk([1u8; 32]);
    let authority = pk([2u8; 32]);
    let buyer = pk([3u8; 32]);
    let provider = pk([4u8; 32]);
    let mint = pk([5u8; 32]);
    let payer = pk([6u8; 32]);
    let key = [7u8; 32];
    let reason = [8u8; 32];
    let now = 1_000i64;

    let init_payload = InitializeConfigPayload {
        genesis_hash: [9u8; 32],
        allowed_mint: mint.to_bytes(),
        token_program: TOKEN_PROGRAM_ID.to_bytes(),
        mint_decimals: 6,
        min_provider_bond: 1_000,
        challenge_duration_seconds: 60,
    };
    let plan = plan_initialize_config(&program, &admin, &init_payload).expect("init");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    let config = config_pda(&program).expect("config").address;
    assert_eq!(ix.accounts.len(), 3);
    assert_account(&ix.accounts[0], &admin, true, true);
    assert_account(&ix.accounts[1], &config, true, false);
    assert_account(&ix.accounts[2], &Pubkey::default(), false, false);
    assert_eq!(ix.data, encode_initialize_config(&init_payload).to_vec());

    let plan = plan_set_paused(&program, &admin, true).expect("pause");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    assert_account(&ix.accounts[0], &admin, false, true);
    assert_account(&ix.accounts[1], &config, true, false);
    assert_eq!(ix.data, encode_set_paused(true).to_vec());

    let plan = plan_register_provider(&program, &authority).expect("register");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    let provider_pda_addr = provider_pda(&program, &authority)
        .expect("provider")
        .address;
    assert_account(&ix.accounts[0], &authority, true, true);
    assert_account(&ix.accounts[1], &config, false, false);
    assert_account(&ix.accounts[2], &provider_pda_addr, true, false);
    assert_account(&ix.accounts[3], &Pubkey::default(), false, false);
    assert_eq!(
        ix.data,
        encode_empty(InstructionKind::RegisterProvider)
            .expect("empty")
            .to_vec()
    );

    let plan = plan_add_execution_key(&program, &authority, &key).expect("add key");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    assert_account(&ix.accounts[0], &authority, false, true);
    assert_account(&ix.accounts[1], &provider_pda_addr, true, false);
    assert_eq!(ix.data, encode_add_execution_key(&key).to_vec());

    let plan = plan_revoke_execution_key(&program, &authority, &key).expect("revoke");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    assert_eq!(ix.data, encode_revoke_execution_key(&key).to_vec());

    let plan = plan_deposit_bond(&program, &authority, &mint, 50).expect("deposit");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    let authority_bond = provider_bond_pda(&program, &authority, &mint)
        .expect("authority bond")
        .address;
    let vault = bond_vault_ata(&authority_bond, &mint);
    let auth_ata = user_settlement_ata(&authority, &mint);
    assert_eq!(ix.accounts.len(), 9);
    assert_account(&ix.accounts[0], &authority, true, true);
    assert_account(&ix.accounts[1], &config, false, false);
    assert_account(&ix.accounts[2], &provider_pda_addr, false, false);
    assert_account(&ix.accounts[3], &authority_bond, true, false);
    assert_account(&ix.accounts[4], &vault, true, false);
    assert_account(&ix.accounts[5], &auth_ata, true, false);
    assert_account(&ix.accounts[6], &mint, false, false);
    assert_account(&ix.accounts[7], &TOKEN_PROGRAM_ID, false, false);
    assert_account(&ix.accounts[8], &Pubkey::default(), false, false);
    assert_eq!(ix.data, encode_deposit_bond(50).to_vec());

    let plan = plan_withdraw_bond(&program, &authority, &mint, 25).expect("withdraw");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    assert_eq!(ix.data, encode_withdraw_bond(25).to_vec());
    assert_account(&ix.accounts[0], &authority, false, true);

    let create_payload = CreateJobPayload {
        job_nonce: 11,
        amount: 100,
        request_hash: [9u8; 32],
        fund_deadline: now + 10,
        accept_deadline: now + 20,
        work_deadline: now + 30,
        auto_settle_deadline: now + 40,
    };
    let plan = plan_create_job(&program, &buyer, &provider, now, &create_payload).expect("create");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    let job = job_pda(&program, &buyer, &provider, 11)
        .expect("job")
        .address;
    let provider_acc = provider_pda(&program, &provider).expect("provider").address;
    let provider_bond = provider_bond_pda(&program, &provider, &mint)
        .expect("provider bond")
        .address;
    assert_account(&ix.accounts[0], &buyer, true, true);
    assert_account(&ix.accounts[1], &config, false, false);
    assert_account(&ix.accounts[2], &provider_acc, false, false);
    assert_account(&ix.accounts[3], &job, true, false);
    assert_account(&ix.accounts[4], &Pubkey::default(), false, false);
    assert_eq!(ix.data, encode_create_job(&create_payload).to_vec());
    assert_eq!(plan.expires_at, Some(create_payload.fund_deadline));

    let plan = plan_fund_job(&program, &buyer, &provider, &mint, 11).expect("fund");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    let buyer_ata = user_settlement_ata(&buyer, &mint);
    let escrow = job_escrow_ata(&job, &mint);
    assert_account(&ix.accounts[0], &buyer, false, true);
    assert_account(&ix.accounts[3], &buyer_ata, true, false);
    assert_account(&ix.accounts[4], &escrow, true, false);
    assert_eq!(
        ix.data,
        encode_empty(InstructionKind::FundJob)
            .expect("empty")
            .to_vec()
    );

    let plan = plan_accept_job(&program, &provider, &buyer, &mint, 11).expect("accept");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    assert_account(&ix.accounts[0], &provider, false, true);
    assert_account(&ix.accounts[3], &provider_bond, true, false);
    assert_account(&ix.accounts[4], &job, true, false);

    let plan = plan_accept_work(&program, &buyer, &provider, &mint, 11).expect("accept work");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    let provider_ata = user_settlement_ata(&provider, &mint);
    assert_account(&ix.accounts[0], &buyer, false, true);
    assert_account(&ix.accounts[4], &provider_ata, true, false);
    assert_account(&ix.accounts[5], &buyer_ata, true, false);

    let plan = plan_challenge_work(&program, &buyer, &provider, 11, &reason).expect("challenge");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    let challenge = challenge_pda(&program, &job).expect("challenge").address;
    assert_account(&ix.accounts[0], &buyer, true, true);
    assert_account(&ix.accounts[3], &challenge, true, false);
    assert_eq!(ix.data, encode_challenge_work(&reason).to_vec());

    let plan = plan_resolve_timeout_settle(&program, &payer, &buyer, &provider, &mint, 11, false)
        .expect("settle");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    assert_eq!(ix.accounts.len(), 9);
    assert_account(&ix.accounts[0], &payer, false, true);
    assert_account(&ix.accounts[6], &buyer, true, false);

    let plan = plan_resolve_timeout_settle(&program, &payer, &buyer, &provider, &mint, 11, true)
        .expect("settle+chal");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    assert_eq!(ix.accounts.len(), 10);
    assert_account(&ix.accounts[9], &challenge, true, false);

    let plan = plan_resolve_timeout_refund(&program, &payer, &buyer, &provider, &mint, 11)
        .expect("refund");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    assert_eq!(
        ix.data,
        encode_empty(InstructionKind::ResolveTimeoutRefund)
            .expect("empty")
            .to_vec()
    );

    let plan = plan_expire_unfunded(&program, &payer, &buyer, &provider, 11).expect("expire");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    assert_eq!(ix.accounts.len(), 2);
    assert_account(&ix.accounts[1], &job, true, false);

    let plan =
        plan_expire_unaccepted(&program, &payer, &buyer, &provider, &mint, 11).expect("unaccepted");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    assert_eq!(ix.accounts.len(), 8);

    let plan = plan_slash_bond(&program, &admin, &buyer, &provider, &mint, 11).expect("slash");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    assert_eq!(ix.accounts.len(), 11);
    assert_account(&ix.accounts[0], &admin, false, true);
    assert_account(&ix.accounts[8], &challenge, true, false);

    let plan = plan_close_job(&program, &buyer, &provider, &mint, 11, false).expect("close");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    assert_eq!(ix.accounts.len(), 3);
    let plan = plan_close_job(&program, &buyer, &provider, &mint, 11, true).expect("close+escrow");
    let ix = &plan.to_solana_instructions().expect("ixs")[0];
    assert_eq!(ix.accounts.len(), 5);
    assert_account(&ix.accounts[3], &escrow, true, false);
    assert_account(&ix.accounts[4], &TOKEN_PROGRAM_ID, false, false);
}

#[test]
fn receipt_digest_parity_and_ed25519_layout() {
    let receipt = sample_receipt();
    validate_receipt(&receipt).expect("validate");
    let digest = receipt_digest(&receipt).expect("digest");
    assert_eq!(digest, receipt.digest().expect("types digest"));

    let encoded = receipt.encode().expect("encode");
    assert_eq!(encoded.len(), RECEIPT_ENCODED_LEN);
    assert_eq!(RECEIPT_ENCODED_LEN, 334);

    let pubkey = [11u8; 32];
    let signature = [22u8; 64];
    let ix = build_ed25519_verify_instruction(&encoded, &pubkey, &signature).expect("ed25519");
    assert!(ix.accounts.is_empty());
    assert_eq!(
        ix.program_id,
        Pubkey::from_str_const("Ed25519SigVerify111111111111111111111111111")
    );

    const OFFSETS_START: usize = 2;
    const OFFSETS_SIZE: usize = 14;
    const DATA_START: usize = OFFSETS_START + OFFSETS_SIZE;
    assert_eq!(ix.data[0], 1);
    assert_eq!(ix.data[1], 0);
    let signature_offset = u16::from_le_bytes([ix.data[2], ix.data[3]]) as usize;
    let public_key_offset = u16::from_le_bytes([ix.data[6], ix.data[7]]) as usize;
    let message_offset = u16::from_le_bytes([ix.data[10], ix.data[11]]) as usize;
    let message_size = u16::from_le_bytes([ix.data[12], ix.data[13]]) as usize;
    assert_eq!(public_key_offset, DATA_START);
    assert_eq!(signature_offset, DATA_START + 32);
    assert_eq!(message_offset, DATA_START + 32 + 64);
    assert_eq!(message_size, 334);
    assert_eq!(&ix.data[public_key_offset..public_key_offset + 32], &pubkey);
    assert_eq!(
        &ix.data[signature_offset..signature_offset + 64],
        &signature
    );
    assert_eq!(
        &ix.data[message_offset..message_offset + 334],
        encoded.as_slice()
    );
    assert_eq!(ix.data.len(), DATA_START + 32 + 64 + 334);

    assert!(build_ed25519_verify_instruction(&[0u8; 10], &pubkey, &signature).is_err());

    let program = program_id();
    let job = pk(receipt.job);
    let provider = pk(receipt.provider);
    let plan = build_submit_receipt_plan(&program, &job, &provider, &receipt, &pubkey, &signature)
        .expect("submit plan");
    assert_eq!(plan.action, "submit_receipt");
    assert_eq!(plan.instructions.len(), 2);
    assert!(plan.required_signers.is_empty());
    let submit = &plan.to_solana_instructions().expect("ixs")[1];
    assert_eq!(
        submit.data,
        encode_submit_receipt(&receipt)
            .expect("encode submit")
            .to_vec()
    );
}

#[test]
fn instruction_plan_json_round_trip() {
    let program = program_id();
    let buyer = pk([3u8; 32]);
    let provider = pk([4u8; 32]);
    let now = 1_000i64;
    let payload = CreateJobPayload {
        job_nonce: 1,
        amount: 100,
        request_hash: [9u8; 32],
        fund_deadline: now + 10,
        accept_deadline: now + 20,
        work_deadline: now + 30,
        auto_settle_deadline: now + 40,
    };
    let plan = plan_create_job(&program, &buyer, &provider, now, &payload).expect("plan");
    let json = plan.to_json().expect("to_json");
    let restored = InstructionPlan::from_json(&json).expect("from_json");
    assert_eq!(restored, plan);

    let ixs = restored.to_solana_instructions().expect("ixs");
    assert_eq!(ixs.len(), 1);
    assert_plan_account(&restored.instructions[0].accounts[0], &buyer, true, true);

    // Sanity: CONFIG_ACCOUNT_LEN unused constant still available for decode tests.
    assert_eq!(CONFIG_ACCOUNT_LEN, 149);
    assert_eq!(PROVIDER_ACCOUNT_LEN, 165);
}
