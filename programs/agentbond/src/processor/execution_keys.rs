use agentbond_types::{
    ProtocolError, ProtocolEventKind, MAX_EXECUTION_KEYS, PROVIDER_STATUS_ACTIVE,
};
use pinocchio::account::AccountView;
use pinocchio::address::Address;
use pinocchio::error::ProgramResult;

use crate::accounts::{require_keys_eq, require_signer, require_writable};
use crate::error::fail;
use crate::events;
use crate::processor::helpers::{load_validated_provider, next_account, now_ts, save_provider};

pub fn add_execution_key(
    program_id: &Address,
    accounts: &[AccountView],
    key: [u8; 32],
) -> ProgramResult {
    let mut accounts = accounts.iter();
    let authority = next_account(&mut accounts)?;
    let provider_account = next_account(&mut accounts)?;

    require_signer(authority)?;
    require_writable(provider_account)?;

    if key == [0u8; 32] {
        return Err(fail(ProtocolError::InvalidPubkey));
    }

    let mut provider = load_validated_provider(program_id, provider_account)?;
    require_keys_eq(
        authority.address(),
        &provider.authority,
        ProtocolError::Unauthorized,
    )?;
    if provider.status != PROVIDER_STATUS_ACTIVE {
        return Err(fail(ProtocolError::ProviderInactive));
    }
    if provider.contains_execution_key(&key) {
        return Err(fail(ProtocolError::DuplicateExecutionKey));
    }
    let count = usize::from(provider.execution_key_count);
    if count >= MAX_EXECUTION_KEYS {
        return Err(fail(ProtocolError::ExecutionKeyLimit));
    }
    provider.execution_keys[count] = key;
    provider.execution_key_count = provider
        .execution_key_count
        .checked_add(1)
        .ok_or_else(|| fail(ProtocolError::MathOverflow))?;
    save_provider(provider_account, &provider)?;

    events::emit(
        ProtocolEventKind::ExecutionKeyAdded,
        provider_account.address(),
        authority.address(),
        0,
        now_ts()?,
    )
}

pub fn revoke_execution_key(
    program_id: &Address,
    accounts: &[AccountView],
    key: [u8; 32],
) -> ProgramResult {
    let mut accounts = accounts.iter();
    let authority = next_account(&mut accounts)?;
    let provider_account = next_account(&mut accounts)?;

    require_signer(authority)?;
    require_writable(provider_account)?;

    let mut provider = load_validated_provider(program_id, provider_account)?;
    require_keys_eq(
        authority.address(),
        &provider.authority,
        ProtocolError::Unauthorized,
    )?;

    let count = usize::from(provider.execution_key_count);
    let mut found = None;
    for index in 0..count {
        if provider.execution_keys[index] == key {
            found = Some(index);
            break;
        }
    }
    let index = found.ok_or_else(|| fail(ProtocolError::ExecutionKeyNotFound))?;

    let last = count
        .checked_sub(1)
        .ok_or_else(|| fail(ProtocolError::MathOverflow))?;
    if index != last {
        provider.execution_keys[index] = provider.execution_keys[last];
    }
    provider.execution_keys[last] = [0u8; 32];
    provider.execution_key_count = provider
        .execution_key_count
        .checked_sub(1)
        .ok_or_else(|| fail(ProtocolError::MathOverflow))?;
    save_provider(provider_account, &provider)?;

    events::emit(
        ProtocolEventKind::ExecutionKeyRevoked,
        provider_account.address(),
        authority.address(),
        0,
        now_ts()?,
    )
}
