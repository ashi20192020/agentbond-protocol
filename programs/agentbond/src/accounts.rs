use agentbond_types::{
    bond_seed_parts, challenge_seed_parts, job_nonce_le_bytes, job_seed_parts, provider_seed_parts,
    ProtocolError, BOND_SEED, CHALLENGE_SEED, CONFIG_SEED, JOB_SEED, PROVIDER_SEED,
};
use pinocchio::account::AccountView;
use pinocchio::address::Address;
use pinocchio::cpi::{Seed, Signer};
use pinocchio::error::ProgramResult;
use pinocchio_system::instructions::CreateAccount;

use crate::constants::SYSTEM_PROGRAM_ID;
use crate::error::fail;

pub fn require_signer(account: &AccountView) -> Result<(), pinocchio::error::ProgramError> {
    if !account.is_signer() {
        return Err(fail(ProtocolError::MissingSignature));
    }
    Ok(())
}

pub fn require_writable(account: &AccountView) -> Result<(), pinocchio::error::ProgramError> {
    if !account.is_writable() {
        return Err(fail(ProtocolError::AccountNotWritable));
    }
    Ok(())
}

pub fn require_readonly(account: &AccountView) -> Result<(), pinocchio::error::ProgramError> {
    if account.is_writable() {
        return Err(fail(ProtocolError::InvalidConfig));
    }
    Ok(())
}

pub fn require_system_program(account: &AccountView) -> Result<(), pinocchio::error::ProgramError> {
    if account.address() != &SYSTEM_PROGRAM_ID {
        return Err(fail(ProtocolError::InvalidOwner));
    }
    Ok(())
}

pub fn address_from_bytes(bytes: &[u8; 32]) -> Address {
    Address::new_from_array(*bytes)
}

pub fn pubkey_eq(account: &AccountView, expected: &[u8; 32]) -> bool {
    account.address().as_ref() == expected
}

pub fn require_keys_eq(
    actual: &Address,
    expected: &[u8; 32],
    error: ProtocolError,
) -> Result<(), pinocchio::error::ProgramError> {
    if actual.as_ref() != expected {
        return Err(fail(error));
    }
    Ok(())
}

pub fn require_account_key(
    account: &AccountView,
    expected: &Address,
    error: ProtocolError,
) -> Result<(), pinocchio::error::ProgramError> {
    if account.address() != expected {
        return Err(fail(error));
    }
    Ok(())
}

pub fn require_owner(
    account: &AccountView,
    owner: &Address,
) -> Result<(), pinocchio::error::ProgramError> {
    if !account.owned_by(owner) {
        return Err(fail(ProtocolError::InvalidOwner));
    }
    Ok(())
}

pub fn require_uninitialized(account: &AccountView) -> Result<(), pinocchio::error::ProgramError> {
    if !account.owned_by(&SYSTEM_PROGRAM_ID) || !account.is_data_empty() {
        return Err(fail(ProtocolError::AlreadyInitialized));
    }
    Ok(())
}

pub fn create_pda_account<'a>(
    payer: &'a AccountView,
    account: &'a AccountView,
    program_id: &Address,
    space: usize,
    signer_seeds: &[Seed],
) -> ProgramResult {
    require_writable(account)?;
    require_uninitialized(account)?;

    let create =
        CreateAccount::with_minimum_balance(payer, account, space as u64, program_id, None)
            .map_err(|_| fail(ProtocolError::InvalidAccountData))?;
    let signer = Signer::from(signer_seeds);
    create.invoke_signed(&[signer])?;
    Ok(())
}

pub fn write_account_data(account: &AccountView, data: &[u8]) -> ProgramResult {
    require_writable(account)?;
    if account.data_len() != data.len() {
        return Err(fail(ProtocolError::InvalidAccountLength));
    }
    let mut dst = account
        .try_borrow_mut()
        .map_err(|_| fail(ProtocolError::InvalidAccountData))?;
    dst.copy_from_slice(data);
    Ok(())
}

pub fn close_pda_account(account: &AccountView, recipient: &AccountView) -> ProgramResult {
    require_writable(account)?;
    require_writable(recipient)?;

    let lamports = account.lamports();
    let recipient_lamports = recipient
        .lamports()
        .checked_add(lamports)
        .ok_or_else(|| fail(ProtocolError::MathOverflow))?;
    recipient.set_lamports(recipient_lamports);
    account.set_lamports(0);

    {
        let mut data = account
            .try_borrow_mut()
            .map_err(|_| fail(ProtocolError::InvalidAccountData))?;
        data.fill(0);
    }

    account.close()?;
    Ok(())
}

pub fn validate_config_pda(
    program_id: &Address,
    account: &AccountView,
    bump: u8,
) -> Result<(), pinocchio::error::ProgramError> {
    let bump_seed = [bump];
    let seeds: &[&[u8]] = &[CONFIG_SEED, &bump_seed];
    let expected = Address::create_program_address(seeds, program_id)
        .map_err(|_| fail(ProtocolError::InvalidPda))?;
    require_account_key(account, &expected, ProtocolError::InvalidPda)?;
    require_owner(account, program_id)?;
    Ok(())
}

pub fn validate_provider_pda(
    program_id: &Address,
    account: &AccountView,
    authority: &[u8; 32],
    bump: u8,
) -> Result<(), pinocchio::error::ProgramError> {
    let bump_seed = [bump];
    let parts = provider_seed_parts(authority);
    let seeds: &[&[u8]] = &[parts[0], parts[1], &bump_seed];
    let expected = Address::create_program_address(seeds, program_id)
        .map_err(|_| fail(ProtocolError::InvalidPda))?;
    require_account_key(account, &expected, ProtocolError::InvalidPda)?;
    require_owner(account, program_id)?;
    Ok(())
}

pub fn validate_bond_pda(
    program_id: &Address,
    account: &AccountView,
    authority: &[u8; 32],
    mint: &[u8; 32],
    bump: u8,
) -> Result<(), pinocchio::error::ProgramError> {
    let bump_seed = [bump];
    let parts = bond_seed_parts(authority, mint);
    let seeds: &[&[u8]] = &[parts[0], parts[1], parts[2], &bump_seed];
    let expected = Address::create_program_address(seeds, program_id)
        .map_err(|_| fail(ProtocolError::InvalidPda))?;
    require_account_key(account, &expected, ProtocolError::InvalidPda)?;
    require_owner(account, program_id)?;
    Ok(())
}

pub fn validate_job_pda(
    program_id: &Address,
    account: &AccountView,
    buyer: &[u8; 32],
    provider: &[u8; 32],
    job_nonce: u64,
    bump: u8,
) -> Result<(), pinocchio::error::ProgramError> {
    let bump_seed = [bump];
    let nonce = job_nonce_le_bytes(job_nonce);
    let parts = job_seed_parts(buyer, provider, &nonce);
    let seeds: &[&[u8]] = &[parts[0], parts[1], parts[2], parts[3], &bump_seed];
    let expected = Address::create_program_address(seeds, program_id)
        .map_err(|_| fail(ProtocolError::InvalidPda))?;
    require_account_key(account, &expected, ProtocolError::InvalidPda)?;
    require_owner(account, program_id)?;
    Ok(())
}

pub fn validate_challenge_pda(
    program_id: &Address,
    account: &AccountView,
    job: &[u8; 32],
    bump: u8,
) -> Result<(), pinocchio::error::ProgramError> {
    let bump_seed = [bump];
    let parts = challenge_seed_parts(job);
    let seeds: &[&[u8]] = &[parts[0], parts[1], &bump_seed];
    let expected = Address::create_program_address(seeds, program_id)
        .map_err(|_| fail(ProtocolError::InvalidPda))?;
    require_account_key(account, &expected, ProtocolError::InvalidPda)?;
    require_owner(account, program_id)?;
    Ok(())
}

pub fn provider_signer_seeds<'a>(authority: &'a [u8; 32], bump: &'a u8) -> [Seed<'a>; 3] {
    [
        Seed::from(PROVIDER_SEED),
        Seed::from(authority.as_slice()),
        Seed::from(core::slice::from_ref(bump)),
    ]
}

pub fn bond_signer_seeds<'a>(
    authority: &'a [u8; 32],
    mint: &'a [u8; 32],
    bump: &'a u8,
) -> [Seed<'a>; 4] {
    [
        Seed::from(BOND_SEED),
        Seed::from(authority.as_slice()),
        Seed::from(mint.as_slice()),
        Seed::from(core::slice::from_ref(bump)),
    ]
}

pub fn job_signer_seeds<'a>(
    buyer: &'a [u8; 32],
    provider: &'a [u8; 32],
    nonce_le: &'a [u8; 8],
    bump: &'a u8,
) -> [Seed<'a>; 5] {
    [
        Seed::from(JOB_SEED),
        Seed::from(buyer.as_slice()),
        Seed::from(provider.as_slice()),
        Seed::from(nonce_le.as_slice()),
        Seed::from(core::slice::from_ref(bump)),
    ]
}

pub fn challenge_signer_seeds<'a>(job: &'a [u8; 32], bump: &'a u8) -> [Seed<'a>; 3] {
    [
        Seed::from(CHALLENGE_SEED),
        Seed::from(job.as_slice()),
        Seed::from(core::slice::from_ref(bump)),
    ]
}
