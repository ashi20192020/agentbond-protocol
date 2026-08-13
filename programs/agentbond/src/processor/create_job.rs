use agentbond_types::{
    CreateJobPayload, JobAccount, JobState, ProtocolError, ProtocolEventKind, JOB_ACCOUNT_LEN,
    JOB_SEED, PROVIDER_STATUS_ACTIVE,
};
use pinocchio::account::AccountView;
use pinocchio::address::Address;
use pinocchio::cpi::Seed;
use pinocchio::error::ProgramResult;

use crate::accounts::{
    create_pda_account, require_signer, require_system_program, require_uninitialized,
};
use crate::error::fail;
use crate::events;
use crate::processor::helpers::{
    load_validated_config_readonly, load_validated_provider, next_account, now_ts,
    require_not_paused, save_job,
};
use crate::token::require_legacy_token_program_id;

pub fn process(
    program_id: &Address,
    accounts: &[AccountView],
    payload: CreateJobPayload,
) -> ProgramResult {
    let mut accounts = accounts.iter();
    let buyer = next_account(&mut accounts)?;
    let config_account = next_account(&mut accounts)?;
    let provider_account = next_account(&mut accounts)?;
    let job_account = next_account(&mut accounts)?;
    let system_program = next_account(&mut accounts)?;

    require_signer(buyer)?;
    require_system_program(system_program)?;

    let config = load_validated_config_readonly(program_id, config_account)?;
    require_not_paused(&config)?;
    require_legacy_token_program_id(&config.token_program)?;

    if payload.amount == 0 {
        return Err(fail(ProtocolError::InvalidAmount));
    }
    if payload.request_hash == [0u8; 32] {
        return Err(fail(ProtocolError::InvalidPubkey));
    }

    let now = now_ts()?;
    if !(now < payload.fund_deadline
        && payload.fund_deadline < payload.accept_deadline
        && payload.accept_deadline < payload.work_deadline
        && payload.work_deadline < payload.auto_settle_deadline)
    {
        return Err(fail(ProtocolError::InvalidDeadlineOrder));
    }

    let provider = load_validated_provider(program_id, provider_account)?;
    if provider.status != PROVIDER_STATUS_ACTIVE {
        return Err(fail(ProtocolError::ProviderInactive));
    }

    let buyer_bytes = buyer.address().to_bytes();
    let nonce = payload.job_nonce.to_le_bytes();
    let (expected, bump) = Address::try_find_program_address(
        &[
            JOB_SEED,
            buyer_bytes.as_slice(),
            provider.authority.as_slice(),
            nonce.as_slice(),
        ],
        program_id,
    )
    .ok_or_else(|| fail(ProtocolError::InvalidPda))?;
    if job_account.address() != &expected {
        return Err(fail(ProtocolError::InvalidPda));
    }
    require_uninitialized(job_account)?;

    let bump_seed = [bump];
    let seeds = [
        Seed::from(JOB_SEED),
        Seed::from(buyer_bytes.as_slice()),
        Seed::from(provider.authority.as_slice()),
        Seed::from(nonce.as_slice()),
        Seed::from(bump_seed.as_slice()),
    ];
    create_pda_account(buyer, job_account, program_id, JOB_ACCOUNT_LEN, &seeds)?;

    let job = JobAccount {
        bump,
        state: JobState::Created,
        buyer: buyer_bytes,
        provider: provider.authority,
        mint: config.allowed_mint,
        token_program: config.token_program,
        amount: payload.amount,
        job_nonce: payload.job_nonce,
        fund_deadline: payload.fund_deadline,
        accept_deadline: payload.accept_deadline,
        work_deadline: payload.work_deadline,
        auto_settle_deadline: payload.auto_settle_deadline,
        receipt_digest: [0u8; 32],
        request_hash: payload.request_hash,
        locked_bond: 0,
        mint_decimals: config.mint_decimals,
    };
    save_job(job_account, &job)?;

    events::emit(
        ProtocolEventKind::JobCreated,
        job_account.address(),
        buyer.address(),
        payload.amount,
        now,
    )
}
