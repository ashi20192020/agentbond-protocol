use agentbond_types::{JobState, ProtocolError, ProtocolEventKind};
use pinocchio::account::AccountView;
use pinocchio::address::Address;
use pinocchio::error::ProgramResult;

use crate::accounts::{require_signer, require_writable};
use crate::error::fail;
use crate::events;
use crate::processor::helpers::{
    load_validated_config, load_validated_job, next_account, now_ts, require_not_paused, save_job,
    transition_job,
};
use crate::token::{
    require_legacy_token_program, token_amount, transfer_checked, validate_ata, validate_mint,
    validate_token_account,
};

pub fn process(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let mut accounts = accounts.iter();
    let buyer = next_account(&mut accounts)?;
    let config_account = next_account(&mut accounts)?;
    let job_account = next_account(&mut accounts)?;
    let buyer_token = next_account(&mut accounts)?;
    let escrow = next_account(&mut accounts)?;
    let mint = next_account(&mut accounts)?;
    let token_program = next_account(&mut accounts)?;

    require_signer(buyer)?;
    require_writable(job_account)?;
    require_legacy_token_program(token_program)?;

    let config = load_validated_config(program_id, config_account)?;
    require_not_paused(&config)?;

    let mut job = load_validated_job(program_id, job_account)?;
    if job.buyer.as_ref() != buyer.address().as_ref() {
        return Err(fail(ProtocolError::Unauthorized));
    }
    if job.state != JobState::Created {
        return Err(fail(ProtocolError::InvalidStateTransition));
    }

    let now = now_ts()?;
    if now >= job.fund_deadline {
        return Err(fail(ProtocolError::DeadlineExpired));
    }

    if job.mint != config.allowed_mint || job.token_program != config.token_program {
        return Err(fail(ProtocolError::InvalidConfig));
    }
    if token_program.address().as_ref() != job.token_program {
        return Err(fail(ProtocolError::InvalidTokenProgram));
    }

    validate_mint(mint, &job.mint, job.mint_decimals)?;
    validate_token_account(buyer_token, &job.mint, buyer.address())?;
    let escrow_balance = validate_ata(escrow, job_account.address(), &job.mint)?;
    if escrow_balance != 0 {
        return Err(fail(ProtocolError::EscrowNotEmpty));
    }

    transfer_checked(
        buyer_token,
        mint,
        escrow,
        buyer,
        job.amount,
        job.mint_decimals,
        None,
    )?;

    if token_amount(escrow)? != job.amount {
        return Err(fail(ProtocolError::EscrowUnexpectedBalance));
    }

    transition_job(&mut job, JobState::Funded)?;
    save_job(job_account, &job)?;

    events::emit(
        ProtocolEventKind::JobFunded,
        job_account.address(),
        buyer.address(),
        job.amount,
        now,
    )
}
