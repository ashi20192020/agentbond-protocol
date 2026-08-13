use agentbond_types::{
    ProtocolError, ProtocolEventKind, ProviderAccount, MAX_EXECUTION_KEYS, PROVIDER_ACCOUNT_LEN,
    PROVIDER_SEED, PROVIDER_STATUS_ACTIVE,
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
    load_validated_config, next_account, now_ts, require_not_paused, save_provider,
};

pub fn process(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let mut accounts = accounts.iter();
    let authority = next_account(&mut accounts)?;
    let config_account = next_account(&mut accounts)?;
    let provider_account = next_account(&mut accounts)?;
    let system_program = next_account(&mut accounts)?;

    require_signer(authority)?;
    require_system_program(system_program)?;

    let config = load_validated_config(program_id, config_account)?;
    require_not_paused(&config)?;

    let authority_bytes = authority.address().to_bytes();
    let (expected, bump) =
        Address::try_find_program_address(&[PROVIDER_SEED, authority_bytes.as_slice()], program_id)
            .ok_or_else(|| fail(ProtocolError::InvalidPda))?;
    if provider_account.address() != &expected {
        return Err(fail(ProtocolError::InvalidPda));
    }
    require_uninitialized(provider_account)?;

    let bump_seed = [bump];
    let seeds = [
        Seed::from(PROVIDER_SEED),
        Seed::from(authority_bytes.as_slice()),
        Seed::from(bump_seed.as_slice()),
    ];
    create_pda_account(
        authority,
        provider_account,
        program_id,
        PROVIDER_ACCOUNT_LEN,
        &seeds,
    )?;

    let provider = ProviderAccount {
        bump,
        status: PROVIDER_STATUS_ACTIVE,
        authority: authority_bytes,
        execution_key_count: 0,
        execution_keys: [[0u8; 32]; MAX_EXECUTION_KEYS],
    };
    save_provider(provider_account, &provider)?;

    events::emit(
        ProtocolEventKind::ProviderRegistered,
        provider_account.address(),
        authority.address(),
        0,
        now_ts()?,
    )
}
