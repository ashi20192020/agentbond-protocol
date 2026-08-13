use agentbond_types::{
    validate_transition, ChallengeAccount, ConfigAccount, JobAccount, JobState, ProtocolError,
    ProviderAccount, ProviderBondAccount, CHALLENGE_ACCOUNT_LEN, CONFIG_ACCOUNT_LEN,
    JOB_ACCOUNT_LEN, PROVIDER_ACCOUNT_LEN, PROVIDER_BOND_ACCOUNT_LEN,
};
use pinocchio::account::AccountView;
use pinocchio::address::Address;
use pinocchio::error::ProgramResult;

use crate::accounts::{
    address_from_bytes, job_signer_seeds, require_keys_eq, require_owner, require_writable,
    validate_bond_pda, validate_challenge_pda, validate_config_pda, validate_job_pda,
    validate_provider_pda, write_account_data,
};
use crate::error::fail;
use crate::token::{
    parse_token_account, require_legacy_token_program_id, transfer_checked, validate_ata,
    validate_mint,
};
use pinocchio::sysvars::clock::Clock;
use pinocchio::sysvars::Sysvar;

pub fn now_ts() -> Result<i64, pinocchio::error::ProgramError> {
    let clock = Clock::get().map_err(|_| fail(ProtocolError::InvalidAccountData))?;
    Ok(clock.unix_timestamp)
}

pub fn load_config(account: &AccountView) -> Result<ConfigAccount, pinocchio::error::ProgramError> {
    if account.data_len() != CONFIG_ACCOUNT_LEN {
        return Err(fail(ProtocolError::InvalidAccountLength));
    }
    let data = account
        .try_borrow()
        .map_err(|_| fail(ProtocolError::InvalidAccountData))?;
    ConfigAccount::decode(&data).map_err(fail)
}

pub fn load_provider(
    account: &AccountView,
) -> Result<ProviderAccount, pinocchio::error::ProgramError> {
    if account.data_len() != PROVIDER_ACCOUNT_LEN {
        return Err(fail(ProtocolError::InvalidAccountLength));
    }
    let data = account
        .try_borrow()
        .map_err(|_| fail(ProtocolError::InvalidAccountData))?;
    ProviderAccount::decode(&data).map_err(fail)
}

pub fn load_bond(
    account: &AccountView,
) -> Result<ProviderBondAccount, pinocchio::error::ProgramError> {
    if account.data_len() != PROVIDER_BOND_ACCOUNT_LEN {
        return Err(fail(ProtocolError::InvalidAccountLength));
    }
    let data = account
        .try_borrow()
        .map_err(|_| fail(ProtocolError::InvalidAccountData))?;
    ProviderBondAccount::decode(&data).map_err(fail)
}

pub fn load_job(account: &AccountView) -> Result<JobAccount, pinocchio::error::ProgramError> {
    if account.data_len() != JOB_ACCOUNT_LEN {
        return Err(fail(ProtocolError::InvalidAccountLength));
    }
    let data = account
        .try_borrow()
        .map_err(|_| fail(ProtocolError::InvalidAccountData))?;
    JobAccount::decode(&data).map_err(fail)
}

pub fn load_challenge(
    account: &AccountView,
) -> Result<ChallengeAccount, pinocchio::error::ProgramError> {
    if account.data_len() != CHALLENGE_ACCOUNT_LEN {
        return Err(fail(ProtocolError::InvalidAccountLength));
    }
    let data = account
        .try_borrow()
        .map_err(|_| fail(ProtocolError::InvalidAccountData))?;
    ChallengeAccount::decode(&data).map_err(fail)
}

pub fn save_config(account: &AccountView, config: &ConfigAccount) -> ProgramResult {
    write_account_data(account, &config.encode())
}

pub fn save_provider(account: &AccountView, provider: &ProviderAccount) -> ProgramResult {
    write_account_data(account, &provider.encode().map_err(fail)?)
}

pub fn save_bond(account: &AccountView, bond: &ProviderBondAccount) -> ProgramResult {
    write_account_data(account, &bond.encode().map_err(fail)?)
}

pub fn save_job(account: &AccountView, job: &JobAccount) -> ProgramResult {
    write_account_data(account, &job.encode())
}

pub fn save_challenge(account: &AccountView, challenge: &ChallengeAccount) -> ProgramResult {
    write_account_data(account, &challenge.encode().map_err(fail)?)
}

pub fn require_not_paused(config: &ConfigAccount) -> Result<(), pinocchio::error::ProgramError> {
    if config.paused {
        return Err(fail(ProtocolError::ProtocolPaused));
    }
    Ok(())
}

pub fn require_admin(
    admin: &AccountView,
    config: &ConfigAccount,
) -> Result<(), pinocchio::error::ProgramError> {
    if !admin.is_signer() {
        return Err(fail(ProtocolError::MissingSignature));
    }
    require_keys_eq(admin.address(), &config.admin, ProtocolError::Unauthorized)
}

pub fn load_validated_config(
    program_id: &Address,
    account: &AccountView,
) -> Result<ConfigAccount, pinocchio::error::ProgramError> {
    require_owner(account, program_id)?;
    let config = load_config(account)?;
    validate_config_pda(program_id, account, config.bump)?;
    Ok(config)
}

pub fn load_validated_provider(
    program_id: &Address,
    account: &AccountView,
) -> Result<ProviderAccount, pinocchio::error::ProgramError> {
    require_owner(account, program_id)?;
    let provider = load_provider(account)?;
    validate_provider_pda(program_id, account, &provider.authority, provider.bump)?;
    Ok(provider)
}

pub fn load_validated_bond(
    program_id: &Address,
    account: &AccountView,
) -> Result<ProviderBondAccount, pinocchio::error::ProgramError> {
    require_owner(account, program_id)?;
    let bond = load_bond(account)?;
    validate_bond_pda(program_id, account, &bond.provider, &bond.mint, bond.bump)?;
    Ok(bond)
}

pub fn load_validated_job(
    program_id: &Address,
    account: &AccountView,
) -> Result<JobAccount, pinocchio::error::ProgramError> {
    require_owner(account, program_id)?;
    let job = load_job(account)?;
    validate_job_pda(
        program_id,
        account,
        &job.buyer,
        &job.provider,
        job.job_nonce,
        job.bump,
    )?;
    Ok(job)
}

pub fn load_validated_challenge(
    program_id: &Address,
    account: &AccountView,
    job: &[u8; 32],
) -> Result<ChallengeAccount, pinocchio::error::ProgramError> {
    require_owner(account, program_id)?;
    let challenge = load_challenge(account)?;
    validate_challenge_pda(program_id, account, job, challenge.bump)?;
    if challenge.job != *job {
        return Err(fail(ProtocolError::InvalidPda));
    }
    Ok(challenge)
}

pub fn transition_job(
    job: &mut JobAccount,
    to: JobState,
) -> Result<(), pinocchio::error::ProgramError> {
    validate_transition(job.state, to).map_err(fail)?;
    job.state = to;
    Ok(())
}

pub fn unlock_job_bond(
    job: &mut JobAccount,
    bond: &mut ProviderBondAccount,
) -> Result<(), pinocchio::error::ProgramError> {
    if job.locked_bond == 0 {
        return Ok(());
    }
    if bond.locked < job.locked_bond {
        return Err(fail(ProtocolError::InvalidAccountData));
    }
    bond.locked = bond
        .locked
        .checked_sub(job.locked_bond)
        .ok_or_else(|| fail(ProtocolError::MathOverflow))?;
    job.locked_bond = 0;
    Ok(())
}

pub fn job_nonce_bytes(job: &JobAccount) -> [u8; 8] {
    job.job_nonce.to_le_bytes()
}

#[allow(clippy::too_many_arguments)]
pub fn refund_principal_to_buyer<'a>(
    job: &JobAccount,
    job_account: &'a AccountView,
    escrow: &'a AccountView,
    mint: &'a AccountView,
    buyer_token: &'a AccountView,
    token_program: &AccountView,
    decimals: u8,
) -> ProgramResult {
    require_writable(job_account)?;
    require_legacy_token_program_id(&job.token_program)?;
    if token_program.address().as_ref() != job.token_program {
        return Err(fail(ProtocolError::InvalidTokenProgram));
    }
    validate_mint(mint, &job.mint, decimals)?;
    validate_ata(escrow, job_account.address(), &job.mint)?;
    validate_ata(buyer_token, &address_from_bytes(&job.buyer), &job.mint)?;

    let nonce = job_nonce_bytes(job);
    let bump = job.bump;
    let seeds = job_signer_seeds(&job.buyer, &job.provider, &nonce, &bump);

    // Principal accounting uses job.amount; surplus dust also returns to buyer.
    let balance = parse_token_account(escrow)?.amount();
    if balance < job.amount {
        return Err(fail(ProtocolError::EscrowUnexpectedBalance));
    }
    transfer_checked(
        escrow,
        mint,
        buyer_token,
        job_account,
        balance,
        decimals,
        Some(&seeds),
    )?;
    Ok(())
}

pub fn next_account<'a>(
    accounts: &mut core::slice::Iter<'a, AccountView>,
) -> Result<&'a AccountView, pinocchio::error::ProgramError> {
    accounts
        .next()
        .ok_or_else(|| fail(ProtocolError::InvalidAccountData))
}
