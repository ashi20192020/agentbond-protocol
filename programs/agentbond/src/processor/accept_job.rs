use agentbond_types::{JobState, ProtocolError, ProtocolEventKind, PROVIDER_STATUS_ACTIVE};
use pinocchio::account::AccountView;
use pinocchio::address::Address;
use pinocchio::error::ProgramResult;

use crate::accounts::{require_signer, require_writable};
use crate::error::fail;
use crate::events;
use crate::processor::helpers::{
    load_validated_bond, load_validated_config, load_validated_job, load_validated_provider,
    next_account, now_ts, require_not_paused, save_bond, save_job, transition_job,
};

pub fn process(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let mut accounts = accounts.iter();
    let authority = next_account(&mut accounts)?;
    let config_account = next_account(&mut accounts)?;
    let provider_account = next_account(&mut accounts)?;
    let bond_account = next_account(&mut accounts)?;
    let job_account = next_account(&mut accounts)?;

    require_signer(authority)?;
    require_writable(bond_account)?;
    require_writable(job_account)?;

    let config = load_validated_config(program_id, config_account)?;
    require_not_paused(&config)?;

    let provider = load_validated_provider(program_id, provider_account)?;
    if provider.authority.as_ref() != authority.address().as_ref() {
        return Err(fail(ProtocolError::Unauthorized));
    }
    if provider.status != PROVIDER_STATUS_ACTIVE {
        return Err(fail(ProtocolError::ProviderInactive));
    }

    let mut job = load_validated_job(program_id, job_account)?;
    if job.provider != provider.authority {
        return Err(fail(ProtocolError::Unauthorized));
    }
    if job.state != JobState::Funded {
        return Err(fail(ProtocolError::InvalidStateTransition));
    }

    let now = now_ts()?;
    if now >= job.accept_deadline {
        return Err(fail(ProtocolError::DeadlineExpired));
    }

    let mut bond = load_validated_bond(program_id, bond_account)?;
    if bond.provider != provider.authority || bond.mint != job.mint {
        return Err(fail(ProtocolError::InvalidPda));
    }
    if bond.token_program != job.token_program {
        return Err(fail(ProtocolError::InvalidTokenProgram));
    }

    let unlocked = bond.unlocked().map_err(fail)?;
    if unlocked < config.min_provider_bond {
        return Err(fail(ProtocolError::InsufficientBond));
    }

    bond.locked = bond
        .locked
        .checked_add(config.min_provider_bond)
        .ok_or_else(|| fail(ProtocolError::MathOverflow))?;
    if bond.locked > bond.deposited {
        return Err(fail(ProtocolError::InvalidAccountData));
    }
    job.locked_bond = config.min_provider_bond;
    transition_job(&mut job, JobState::Accepted)?;

    save_bond(bond_account, &bond)?;
    save_job(job_account, &job)?;

    events::emit(
        ProtocolEventKind::JobAccepted,
        job_account.address(),
        authority.address(),
        job.locked_bond,
        now,
    )
}
