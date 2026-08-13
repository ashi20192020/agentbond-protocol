use agentbond_types::{
    ChallengeAccount, JobState, ProtocolError, ProtocolEventKind, PROVIDER_BOND_ACCOUNT_LEN,
};
use pinocchio::account::AccountView;
use pinocchio::address::Address;
use pinocchio::error::ProgramResult;

use crate::accounts::{
    address_from_bytes, bond_signer_seeds, close_pda_account, job_signer_seeds, require_signer,
    require_writable,
};
use crate::constants::SYSTEM_PROGRAM_ID;
use crate::error::fail;
use crate::events;
use crate::processor::helpers::{
    job_nonce_bytes, load_validated_bond, load_validated_challenge, load_validated_config,
    load_validated_job, next_account, now_ts, refund_principal_to_buyer, save_bond, save_job,
    transition_job, unlock_job_bond,
};
use crate::token::{
    require_legacy_token_program, return_surplus_to_buyer, token_amount, transfer_checked,
    validate_ata, validate_mint,
};

pub fn resolve_timeout_settle(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let mut accounts = accounts.iter();
    let _payer = next_account(&mut accounts)?;
    let job_account = next_account(&mut accounts)?;
    let bond_account = next_account(&mut accounts)?;
    let escrow = next_account(&mut accounts)?;
    let provider_token = next_account(&mut accounts)?;
    let buyer_token = next_account(&mut accounts)?;
    let buyer = next_account(&mut accounts)?;
    let mint = next_account(&mut accounts)?;
    let token_program = next_account(&mut accounts)?;
    let challenge_account = accounts.next();

    require_writable(job_account)?;
    require_writable(bond_account)?;
    require_legacy_token_program(token_program)?;

    let mut job = load_validated_job(program_id, job_account)?;
    let now = now_ts()?;

    match job.state {
        JobState::Submitted => {
            if now < job.auto_settle_deadline {
                return Err(fail(ProtocolError::DeadlineNotReached));
            }
        }
        JobState::Challenged => {
            let challenge_account =
                challenge_account.ok_or_else(|| fail(ProtocolError::InvalidAccountData))?;
            let challenge = load_validated_challenge(
                program_id,
                challenge_account,
                &job_account.address().to_bytes(),
            )?;
            if challenge.status != ChallengeAccount::STATUS_OPEN {
                return Err(fail(ProtocolError::InvalidChallengeStatus));
            }
            if now < challenge.deadline {
                return Err(fail(ProtocolError::DeadlineNotReached));
            }
            if buyer.address().as_ref() != challenge.buyer.as_ref() {
                return Err(fail(ProtocolError::InvalidRentRecipient));
            }
            require_writable(challenge_account)?;
            close_pda_account(challenge_account, buyer)?;
        }
        _ => return Err(fail(ProtocolError::InvalidStateTransition)),
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
    validate_ata(buyer_token, &address_from_bytes(&job.buyer), &job.mint)?;
    if buyer.address().as_ref() != job.buyer.as_ref() {
        return Err(fail(ProtocolError::InvalidRentRecipient));
    }

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
        &address_from_bytes(&job.provider),
        amount,
        now,
    )
}

pub fn resolve_timeout_refund(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    refund_internal(program_id, accounts, false)
}

pub fn expire_unaccepted(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    refund_internal(program_id, accounts, true)
}

fn refund_internal(
    program_id: &Address,
    accounts: &[AccountView],
    funded_only: bool,
) -> ProgramResult {
    let mut accounts = accounts.iter();
    let _payer = next_account(&mut accounts)?;
    let job_account = next_account(&mut accounts)?;
    let bond_account = next_account(&mut accounts)?;
    let escrow = next_account(&mut accounts)?;
    let buyer_token = next_account(&mut accounts)?;
    let buyer = next_account(&mut accounts)?;
    let mint = next_account(&mut accounts)?;
    let token_program = next_account(&mut accounts)?;

    require_writable(job_account)?;
    require_legacy_token_program(token_program)?;

    let mut job = load_validated_job(program_id, job_account)?;
    let now = now_ts()?;

    match job.state {
        JobState::Funded => {
            if now < job.accept_deadline {
                return Err(fail(ProtocolError::DeadlineNotReached));
            }
        }
        JobState::Accepted if !funded_only => {
            if now <= job.work_deadline {
                return Err(fail(ProtocolError::DeadlineNotReached));
            }
        }
        _ => return Err(fail(ProtocolError::InvalidStateTransition)),
    }

    if buyer.address().as_ref() != job.buyer.as_ref() {
        return Err(fail(ProtocolError::InvalidRentRecipient));
    }

    refund_principal_to_buyer(
        &job,
        job_account,
        escrow,
        mint,
        buyer_token,
        token_program,
        job.mint_decimals,
    )?;

    let mut bond = if bond_account.data_len() == PROVIDER_BOND_ACCOUNT_LEN
        && bond_account.owned_by(program_id)
    {
        require_writable(bond_account)?;
        let mut bond = load_validated_bond(program_id, bond_account)?;
        if bond.provider == job.provider && bond.mint == job.mint {
            unlock_job_bond(&mut job, &mut bond)?;
            Some(bond)
        } else if job.locked_bond == 0 {
            None
        } else {
            return Err(fail(ProtocolError::InvalidPda));
        }
    } else if job.locked_bond == 0 {
        None
    } else {
        return Err(fail(ProtocolError::InvalidAccountData));
    };

    let amount = job.amount;
    transition_job(&mut job, JobState::Refunded)?;
    if let Some(bond) = bond.as_mut() {
        save_bond(bond_account, bond)?;
    }
    save_job(job_account, &job)?;

    events::emit(
        ProtocolEventKind::JobRefunded,
        job_account.address(),
        buyer.address(),
        amount,
        now,
    )
}

pub fn expire_unfunded(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let mut accounts = accounts.iter();
    let _payer = next_account(&mut accounts)?;
    let job_account = next_account(&mut accounts)?;

    require_writable(job_account)?;
    let mut job = load_validated_job(program_id, job_account)?;
    if job.state != JobState::Created {
        return Err(fail(ProtocolError::InvalidStateTransition));
    }
    let now = now_ts()?;
    if now < job.fund_deadline {
        return Err(fail(ProtocolError::DeadlineNotReached));
    }
    transition_job(&mut job, JobState::Expired)?;
    save_job(job_account, &job)?;

    events::emit(
        ProtocolEventKind::JobExpired,
        job_account.address(),
        &address_from_bytes(&job.buyer),
        0,
        now,
    )
}

pub fn slash_bond(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let mut accounts = accounts.iter();
    let admin = next_account(&mut accounts)?;
    let config_account = next_account(&mut accounts)?;
    let job_account = next_account(&mut accounts)?;
    let bond_account = next_account(&mut accounts)?;
    let bond_vault = next_account(&mut accounts)?;
    let escrow = next_account(&mut accounts)?;
    let buyer_token = next_account(&mut accounts)?;
    let buyer = next_account(&mut accounts)?;
    let challenge_account = next_account(&mut accounts)?;
    let mint = next_account(&mut accounts)?;
    let token_program = next_account(&mut accounts)?;

    require_signer(admin)?;
    require_writable(job_account)?;
    require_writable(bond_account)?;
    require_writable(challenge_account)?;
    require_legacy_token_program(token_program)?;

    let config = load_validated_config(program_id, config_account)?;
    if admin.address().as_ref() != config.admin.as_ref() {
        return Err(fail(ProtocolError::Unauthorized));
    }

    let mut job = load_validated_job(program_id, job_account)?;
    if job.state != JobState::Challenged {
        return Err(fail(ProtocolError::InvalidStateTransition));
    }

    let challenge = load_validated_challenge(
        program_id,
        challenge_account,
        &job_account.address().to_bytes(),
    )?;
    if challenge.status != ChallengeAccount::STATUS_OPEN {
        return Err(fail(ProtocolError::InvalidChallengeStatus));
    }
    let now = now_ts()?;
    if now >= challenge.deadline {
        return Err(fail(ProtocolError::DeadlineExpired));
    }
    if buyer.address().as_ref() != job.buyer.as_ref() {
        return Err(fail(ProtocolError::InvalidRentRecipient));
    }

    refund_principal_to_buyer(
        &job,
        job_account,
        escrow,
        mint,
        buyer_token,
        token_program,
        job.mint_decimals,
    )?;

    let mut bond = load_validated_bond(program_id, bond_account)?;
    if bond.provider != job.provider || bond.mint != job.mint {
        return Err(fail(ProtocolError::InvalidPda));
    }
    let slash_amount = job.locked_bond;
    if slash_amount == 0 {
        return Err(fail(ProtocolError::InvalidAmount));
    }
    if bond.locked < slash_amount || bond.deposited < slash_amount {
        return Err(fail(ProtocolError::InsufficientBond));
    }

    validate_ata(bond_vault, bond_account.address(), &bond.mint)?;
    validate_mint(mint, &job.mint, job.mint_decimals)?;
    let bump = bond.bump;
    let seeds = bond_signer_seeds(&bond.provider, &bond.mint, &bump);
    transfer_checked(
        bond_vault,
        mint,
        buyer_token,
        bond_account,
        slash_amount,
        job.mint_decimals,
        Some(&seeds),
    )?;

    bond.locked = bond
        .locked
        .checked_sub(slash_amount)
        .ok_or_else(|| fail(ProtocolError::MathOverflow))?;
    bond.deposited = bond
        .deposited
        .checked_sub(slash_amount)
        .ok_or_else(|| fail(ProtocolError::MathOverflow))?;
    job.locked_bond = 0;

    transition_job(&mut job, JobState::Slashed)?;
    save_bond(bond_account, &bond)?;
    save_job(job_account, &job)?;
    close_pda_account(challenge_account, buyer)?;

    events::emit(
        ProtocolEventKind::JobSlashed,
        job_account.address(),
        admin.address(),
        slash_amount,
        now,
    )
}

pub fn close_job(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let mut accounts = accounts.iter();
    let buyer = next_account(&mut accounts)?;
    let job_account = next_account(&mut accounts)?;
    let rent_recipient = next_account(&mut accounts)?;
    let escrow = accounts.next();
    let token_program = accounts.next();

    require_signer(buyer)?;
    require_writable(job_account)?;

    let job = load_validated_job(program_id, job_account)?;
    if job.buyer.as_ref() != buyer.address().as_ref() {
        return Err(fail(ProtocolError::Unauthorized));
    }
    if !job.state.is_terminal() {
        return Err(fail(ProtocolError::InvalidStateTransition));
    }
    if rent_recipient.address() != buyer.address() {
        return Err(fail(ProtocolError::InvalidRentRecipient));
    }

    if let Some(escrow) = escrow {
        if !escrow.owned_by(&SYSTEM_PROGRAM_ID) && escrow.data_len() > 0 {
            let token_program =
                token_program.ok_or_else(|| fail(ProtocolError::InvalidTokenProgram))?;
            require_legacy_token_program(token_program)?;
            if token_amount(escrow)? != 0 {
                return Err(fail(ProtocolError::EscrowNotEmpty));
            }
            let nonce = job_nonce_bytes(&job);
            let bump = job.bump;
            let seeds = job_signer_seeds(&job.buyer, &job.provider, &nonce, &bump);
            crate::token::close_token_account(escrow, rent_recipient, job_account, &seeds)?;
        }
    }

    let amount = job.amount;
    close_pda_account(job_account, rent_recipient)?;

    events::emit(
        ProtocolEventKind::JobClosed,
        &address_from_bytes(&job.buyer),
        buyer.address(),
        amount,
        now_ts()?,
    )
}
