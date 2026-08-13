use agentbond_types::{
    CreateJobPayload, InitializeConfigPayload, InstructionKind, encode_add_execution_key,
    encode_challenge_work, encode_create_job, encode_deposit_bond, encode_empty,
    encode_initialize_config, encode_revoke_execution_key, encode_set_paused, encode_withdraw_bond,
};
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use spl_token::ID as TOKEN_PROGRAM_ID;

use crate::address::{
    bond_vault_ata, challenge_pda, config_pda, job_escrow_ata, job_pda, provider_bond_pda,
    provider_pda, user_settlement_ata,
};
use crate::error::SdkError;
use crate::plan::InstructionPlan;

fn system_program() -> Pubkey {
    Pubkey::default()
}

fn empty(kind: InstructionKind) -> Result<Vec<u8>, SdkError> {
    Ok(encode_empty(kind)?.to_vec())
}

fn require_amount(amount: u64) -> Result<(), SdkError> {
    if amount == 0 {
        return Err(SdkError::InvalidAmount);
    }
    Ok(())
}

fn require_deadline_order(
    now: i64,
    fund: i64,
    accept: i64,
    work: i64,
    auto: i64,
) -> Result<(), SdkError> {
    if !(now < fund && fund < accept && accept < work && work < auto) {
        return Err(SdkError::InvalidDeadlineOrder);
    }
    Ok(())
}

pub fn plan_initialize_config(
    program_id: &Pubkey,
    admin: &Pubkey,
    payload: &InitializeConfigPayload,
) -> Result<InstructionPlan, SdkError> {
    if payload.min_provider_bond == 0 {
        return Err(SdkError::InvalidAmount);
    }
    if payload.challenge_duration_seconds <= 0 {
        return Err(SdkError::InvalidInput(
            "challenge_duration must be > 0".into(),
        ));
    }
    let config = config_pda(program_id)?.address;
    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*admin, true),
            AccountMeta::new(config, false),
            AccountMeta::new_readonly(system_program(), false),
        ],
        data: encode_initialize_config(payload).to_vec(),
    };
    Ok(InstructionPlan::new(
        "initialize_config",
        program_id,
        vec![ix],
        vec![*admin],
        None,
    ))
}

pub fn plan_set_paused(
    program_id: &Pubkey,
    admin: &Pubkey,
    paused: bool,
) -> Result<InstructionPlan, SdkError> {
    let config = config_pda(program_id)?.address;
    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new_readonly(*admin, true),
            AccountMeta::new(config, false),
        ],
        data: encode_set_paused(paused).to_vec(),
    };
    Ok(InstructionPlan::new(
        "set_paused",
        program_id,
        vec![ix],
        vec![*admin],
        None,
    ))
}

pub fn plan_register_provider(
    program_id: &Pubkey,
    authority: &Pubkey,
) -> Result<InstructionPlan, SdkError> {
    let config = config_pda(program_id)?.address;
    let provider = provider_pda(program_id, authority)?.address;
    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new_readonly(config, false),
            AccountMeta::new(provider, false),
            AccountMeta::new_readonly(system_program(), false),
        ],
        data: empty(InstructionKind::RegisterProvider)?,
    };
    Ok(InstructionPlan::new(
        "register_provider",
        program_id,
        vec![ix],
        vec![*authority],
        None,
    ))
}

pub fn plan_add_execution_key(
    program_id: &Pubkey,
    authority: &Pubkey,
    key: &[u8; 32],
) -> Result<InstructionPlan, SdkError> {
    if *key == [0u8; 32] {
        return Err(SdkError::InvalidInput(
            "execution key must be nonzero".into(),
        ));
    }
    let provider = provider_pda(program_id, authority)?.address;
    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new_readonly(*authority, true),
            AccountMeta::new(provider, false),
        ],
        data: encode_add_execution_key(key).to_vec(),
    };
    Ok(InstructionPlan::new(
        "add_execution_key",
        program_id,
        vec![ix],
        vec![*authority],
        None,
    ))
}

pub fn plan_revoke_execution_key(
    program_id: &Pubkey,
    authority: &Pubkey,
    key: &[u8; 32],
) -> Result<InstructionPlan, SdkError> {
    let provider = provider_pda(program_id, authority)?.address;
    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new_readonly(*authority, true),
            AccountMeta::new(provider, false),
        ],
        data: encode_revoke_execution_key(key).to_vec(),
    };
    Ok(InstructionPlan::new(
        "revoke_execution_key",
        program_id,
        vec![ix],
        vec![*authority],
        None,
    ))
}

pub fn plan_deposit_bond(
    program_id: &Pubkey,
    authority: &Pubkey,
    mint: &Pubkey,
    amount: u64,
) -> Result<InstructionPlan, SdkError> {
    require_amount(amount)?;
    let config = config_pda(program_id)?.address;
    let provider = provider_pda(program_id, authority)?.address;
    let bond = provider_bond_pda(program_id, authority, mint)?.address;
    let vault = bond_vault_ata(&bond, mint);
    let authority_ata = user_settlement_ata(authority, mint);
    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new_readonly(config, false),
            AccountMeta::new_readonly(provider, false),
            AccountMeta::new(bond, false),
            AccountMeta::new(vault, false),
            AccountMeta::new(authority_ata, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(system_program(), false),
        ],
        data: encode_deposit_bond(amount).to_vec(),
    };
    Ok(InstructionPlan::new(
        "deposit_bond",
        program_id,
        vec![ix],
        vec![*authority],
        None,
    ))
}

pub fn plan_withdraw_bond(
    program_id: &Pubkey,
    authority: &Pubkey,
    mint: &Pubkey,
    amount: u64,
) -> Result<InstructionPlan, SdkError> {
    require_amount(amount)?;
    let bond = provider_bond_pda(program_id, authority, mint)?.address;
    let vault = bond_vault_ata(&bond, mint);
    let authority_ata = user_settlement_ata(authority, mint);
    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new_readonly(*authority, true),
            AccountMeta::new(bond, false),
            AccountMeta::new(vault, false),
            AccountMeta::new(authority_ata, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data: encode_withdraw_bond(amount).to_vec(),
    };
    Ok(InstructionPlan::new(
        "withdraw_bond",
        program_id,
        vec![ix],
        vec![*authority],
        None,
    ))
}

pub fn plan_create_job(
    program_id: &Pubkey,
    buyer: &Pubkey,
    provider_authority: &Pubkey,
    now: i64,
    payload: &CreateJobPayload,
) -> Result<InstructionPlan, SdkError> {
    require_amount(payload.amount)?;
    require_deadline_order(
        now,
        payload.fund_deadline,
        payload.accept_deadline,
        payload.work_deadline,
        payload.auto_settle_deadline,
    )?;
    if payload.request_hash == [0u8; 32] {
        return Err(SdkError::InvalidInput(
            "request_hash must be nonzero".into(),
        ));
    }
    let config = config_pda(program_id)?.address;
    let provider = provider_pda(program_id, provider_authority)?.address;
    let job = job_pda(program_id, buyer, provider_authority, payload.job_nonce)?.address;
    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*buyer, true),
            AccountMeta::new_readonly(config, false),
            AccountMeta::new_readonly(provider, false),
            AccountMeta::new(job, false),
            AccountMeta::new_readonly(system_program(), false),
        ],
        data: encode_create_job(payload).to_vec(),
    };
    Ok(InstructionPlan::new(
        "create_job",
        program_id,
        vec![ix],
        vec![*buyer],
        Some(payload.fund_deadline),
    ))
}

pub fn plan_fund_job(
    program_id: &Pubkey,
    buyer: &Pubkey,
    provider_authority: &Pubkey,
    mint: &Pubkey,
    nonce: u64,
) -> Result<InstructionPlan, SdkError> {
    let config = config_pda(program_id)?.address;
    let job = job_pda(program_id, buyer, provider_authority, nonce)?.address;
    let buyer_ata = user_settlement_ata(buyer, mint);
    let escrow = job_escrow_ata(&job, mint);
    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new_readonly(*buyer, true),
            AccountMeta::new_readonly(config, false),
            AccountMeta::new(job, false),
            AccountMeta::new(buyer_ata, false),
            AccountMeta::new(escrow, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data: empty(InstructionKind::FundJob)?,
    };
    Ok(InstructionPlan::new(
        "fund_job",
        program_id,
        vec![ix],
        vec![*buyer],
        None,
    ))
}

pub fn plan_accept_job(
    program_id: &Pubkey,
    provider_authority: &Pubkey,
    buyer: &Pubkey,
    mint: &Pubkey,
    nonce: u64,
) -> Result<InstructionPlan, SdkError> {
    let config = config_pda(program_id)?.address;
    let provider = provider_pda(program_id, provider_authority)?.address;
    let bond = provider_bond_pda(program_id, provider_authority, mint)?.address;
    let job = job_pda(program_id, buyer, provider_authority, nonce)?.address;
    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new_readonly(*provider_authority, true),
            AccountMeta::new_readonly(config, false),
            AccountMeta::new_readonly(provider, false),
            AccountMeta::new(bond, false),
            AccountMeta::new(job, false),
        ],
        data: empty(InstructionKind::AcceptJob)?,
    };
    Ok(InstructionPlan::new(
        "accept_job",
        program_id,
        vec![ix],
        vec![*provider_authority],
        None,
    ))
}

pub fn plan_accept_work(
    program_id: &Pubkey,
    buyer: &Pubkey,
    provider_authority: &Pubkey,
    mint: &Pubkey,
    nonce: u64,
) -> Result<InstructionPlan, SdkError> {
    let job = job_pda(program_id, buyer, provider_authority, nonce)?.address;
    let bond = provider_bond_pda(program_id, provider_authority, mint)?.address;
    let escrow = job_escrow_ata(&job, mint);
    let provider_ata = user_settlement_ata(provider_authority, mint);
    let buyer_ata = user_settlement_ata(buyer, mint);
    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new_readonly(*buyer, true),
            AccountMeta::new(job, false),
            AccountMeta::new(bond, false),
            AccountMeta::new(escrow, false),
            AccountMeta::new(provider_ata, false),
            AccountMeta::new(buyer_ata, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data: empty(InstructionKind::AcceptWork)?,
    };
    Ok(InstructionPlan::new(
        "accept_work",
        program_id,
        vec![ix],
        vec![*buyer],
        None,
    ))
}

pub fn plan_challenge_work(
    program_id: &Pubkey,
    buyer: &Pubkey,
    provider_authority: &Pubkey,
    nonce: u64,
    reason_hash: &[u8; 32],
) -> Result<InstructionPlan, SdkError> {
    let config = config_pda(program_id)?.address;
    let job = job_pda(program_id, buyer, provider_authority, nonce)?.address;
    let challenge = challenge_pda(program_id, &job)?.address;
    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*buyer, true),
            AccountMeta::new_readonly(config, false),
            AccountMeta::new(job, false),
            AccountMeta::new(challenge, false),
            AccountMeta::new_readonly(system_program(), false),
        ],
        data: encode_challenge_work(reason_hash).to_vec(),
    };
    Ok(InstructionPlan::new(
        "challenge_work",
        program_id,
        vec![ix],
        vec![*buyer],
        None,
    ))
}

pub fn plan_resolve_timeout_settle(
    program_id: &Pubkey,
    payer: &Pubkey,
    buyer: &Pubkey,
    provider_authority: &Pubkey,
    mint: &Pubkey,
    nonce: u64,
    with_challenge: bool,
) -> Result<InstructionPlan, SdkError> {
    let job = job_pda(program_id, buyer, provider_authority, nonce)?.address;
    let bond = provider_bond_pda(program_id, provider_authority, mint)?.address;
    let escrow = job_escrow_ata(&job, mint);
    let provider_ata = user_settlement_ata(provider_authority, mint);
    let buyer_ata = user_settlement_ata(buyer, mint);
    let mut accounts = vec![
        AccountMeta::new_readonly(*payer, true),
        AccountMeta::new(job, false),
        AccountMeta::new(bond, false),
        AccountMeta::new(escrow, false),
        AccountMeta::new(provider_ata, false),
        AccountMeta::new(buyer_ata, false),
        AccountMeta::new(*buyer, false),
        AccountMeta::new_readonly(*mint, false),
        AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
    ];
    if with_challenge {
        accounts.push(AccountMeta::new(
            challenge_pda(program_id, &job)?.address,
            false,
        ));
    }
    let ix = Instruction {
        program_id: *program_id,
        accounts,
        data: empty(InstructionKind::ResolveTimeoutSettle)?,
    };
    Ok(InstructionPlan::new(
        "resolve_timeout_settle",
        program_id,
        vec![ix],
        vec![*payer],
        None,
    ))
}

pub fn plan_resolve_timeout_refund(
    program_id: &Pubkey,
    payer: &Pubkey,
    buyer: &Pubkey,
    provider_authority: &Pubkey,
    mint: &Pubkey,
    nonce: u64,
) -> Result<InstructionPlan, SdkError> {
    let job = job_pda(program_id, buyer, provider_authority, nonce)?.address;
    let bond = provider_bond_pda(program_id, provider_authority, mint)?.address;
    let escrow = job_escrow_ata(&job, mint);
    let buyer_ata = user_settlement_ata(buyer, mint);
    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new_readonly(*payer, true),
            AccountMeta::new(job, false),
            AccountMeta::new(bond, false),
            AccountMeta::new(escrow, false),
            AccountMeta::new(buyer_ata, false),
            AccountMeta::new(*buyer, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data: empty(InstructionKind::ResolveTimeoutRefund)?,
    };
    Ok(InstructionPlan::new(
        "resolve_timeout_refund",
        program_id,
        vec![ix],
        vec![*payer],
        None,
    ))
}

pub fn plan_expire_unfunded(
    program_id: &Pubkey,
    payer: &Pubkey,
    buyer: &Pubkey,
    provider_authority: &Pubkey,
    nonce: u64,
) -> Result<InstructionPlan, SdkError> {
    let job = job_pda(program_id, buyer, provider_authority, nonce)?.address;
    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new_readonly(*payer, true),
            AccountMeta::new(job, false),
        ],
        data: empty(InstructionKind::ExpireUnfunded)?,
    };
    Ok(InstructionPlan::new(
        "expire_unfunded",
        program_id,
        vec![ix],
        vec![*payer],
        None,
    ))
}

pub fn plan_expire_unaccepted(
    program_id: &Pubkey,
    payer: &Pubkey,
    buyer: &Pubkey,
    provider_authority: &Pubkey,
    mint: &Pubkey,
    nonce: u64,
) -> Result<InstructionPlan, SdkError> {
    let job = job_pda(program_id, buyer, provider_authority, nonce)?.address;
    let bond = provider_bond_pda(program_id, provider_authority, mint)?.address;
    let escrow = job_escrow_ata(&job, mint);
    let buyer_ata = user_settlement_ata(buyer, mint);
    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new_readonly(*payer, true),
            AccountMeta::new(job, false),
            AccountMeta::new(bond, false),
            AccountMeta::new(escrow, false),
            AccountMeta::new(buyer_ata, false),
            AccountMeta::new(*buyer, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data: empty(InstructionKind::ExpireUnaccepted)?,
    };
    Ok(InstructionPlan::new(
        "expire_unaccepted",
        program_id,
        vec![ix],
        vec![*payer],
        None,
    ))
}

pub fn plan_slash_bond(
    program_id: &Pubkey,
    admin: &Pubkey,
    buyer: &Pubkey,
    provider_authority: &Pubkey,
    mint: &Pubkey,
    nonce: u64,
) -> Result<InstructionPlan, SdkError> {
    let config = config_pda(program_id)?.address;
    let job = job_pda(program_id, buyer, provider_authority, nonce)?.address;
    let bond = provider_bond_pda(program_id, provider_authority, mint)?.address;
    let vault = bond_vault_ata(&bond, mint);
    let escrow = job_escrow_ata(&job, mint);
    let buyer_ata = user_settlement_ata(buyer, mint);
    let challenge = challenge_pda(program_id, &job)?.address;
    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new_readonly(*admin, true),
            AccountMeta::new_readonly(config, false),
            AccountMeta::new(job, false),
            AccountMeta::new(bond, false),
            AccountMeta::new(vault, false),
            AccountMeta::new(escrow, false),
            AccountMeta::new(buyer_ata, false),
            AccountMeta::new(*buyer, false),
            AccountMeta::new(challenge, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data: empty(InstructionKind::SlashBond)?,
    };
    Ok(InstructionPlan::new(
        "slash_bond",
        program_id,
        vec![ix],
        vec![*admin],
        None,
    ))
}

pub fn plan_close_job(
    program_id: &Pubkey,
    buyer: &Pubkey,
    provider_authority: &Pubkey,
    mint: &Pubkey,
    nonce: u64,
    include_escrow: bool,
) -> Result<InstructionPlan, SdkError> {
    let job = job_pda(program_id, buyer, provider_authority, nonce)?.address;
    let mut accounts = vec![
        AccountMeta::new_readonly(*buyer, true),
        AccountMeta::new(job, false),
        AccountMeta::new(*buyer, false),
    ];
    if include_escrow {
        accounts.push(AccountMeta::new(job_escrow_ata(&job, mint), false));
        accounts.push(AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false));
    }
    let ix = Instruction {
        program_id: *program_id,
        accounts,
        data: empty(InstructionKind::CloseJob)?,
    };
    Ok(InstructionPlan::new(
        "close_job",
        program_id,
        vec![ix],
        vec![*buyer],
        None,
    ))
}
