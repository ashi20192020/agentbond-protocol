use agentbond_types::{JobState, ProtocolError, ProtocolEventKind};
use pinocchio::account::AccountView;
use pinocchio::address::Address;
use pinocchio::error::ProgramResult;

use crate::accounts::{address_from_bytes, job_signer_seeds, require_signer, require_writable};
use crate::error::fail;
use crate::events;
use crate::processor::helpers::{
    job_nonce_bytes, load_validated_bond, load_validated_job, next_account, now_ts, save_bond,
    save_job, transition_job, unlock_job_bond,
};
use crate::token::{
    require_legacy_token_program, return_surplus_to_buyer, token_amount, transfer_checked,
    validate_ata, validate_mint,
};

pub fn process(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let mut accounts = accounts.iter();
    let buyer = next_account(&mut accounts)?;
    let job_account = next_account(&mut accounts)?;
    let bond_account = next_account(&mut accounts)?;
    let escrow = next_account(&mut accounts)?;
    let provider_token = next_account(&mut accounts)?;
    let buyer_token = next_account(&mut accounts)?;
    let mint = next_account(&mut accounts)?;
    let token_program = next_account(&mut accounts)?;

    require_signer(buyer)?;
    require_writable(job_account)?;
    require_writable(bond_account)?;
    require_legacy_token_program(token_program)?;

    let mut job = load_validated_job(program_id, job_account)?;
    if job.buyer.as_ref() != buyer.address().as_ref() {
        return Err(fail(ProtocolError::Unauthorized));
    }
    if job.state != JobState::Submitted {
        return Err(fail(ProtocolError::InvalidStateTransition));
    }
    if token_program.address().as_ref() != job.token_program {
        return Err(fail(ProtocolError::InvalidTokenProgram));
    }

    validate_mint(mint, &job.mint, job.mint_decimals)?;
    validate_ata(escrow, job_account.address(), &job.mint)?;
    validate_ata(
        provider_token,
        &address_from_bytes(&job.provider),
        &job.mint,
    )?;
    validate_ata(buyer_token, buyer.address(), &job.mint)?;

    let mut bond = load_validated_bond(program_id, bond_account)?;
    if bond.provider != job.provider || bond.mint != job.mint {
        return Err(fail(ProtocolError::InvalidPda));
    }

    let nonce = job_nonce_bytes(&job);
    let bump = job.bump;
    let seeds = job_signer_seeds(&job.buyer, &job.provider, &nonce, &bump);

    return_surplus_to_buyer(
        escrow,
        mint,
        buyer_token,
        job_account,
        job.amount,
        job.mint_decimals,
        &seeds,
    )?;
    transfer_checked(
        escrow,
        mint,
        provider_token,
        job_account,
        job.amount,
        job.mint_decimals,
        Some(&seeds),
    )?;
    if token_amount(escrow)? != 0 {
        return Err(fail(ProtocolError::EscrowNotEmpty));
    }

    let amount = job.amount;
    unlock_job_bond(&mut job, &mut bond)?;
    transition_job(&mut job, JobState::Settled)?;
    save_bond(bond_account, &bond)?;
    save_job(job_account, &job)?;

    events::emit(
        ProtocolEventKind::JobSettled,
        job_account.address(),
        buyer.address(),
        amount,
        now_ts()?,
    )
}
