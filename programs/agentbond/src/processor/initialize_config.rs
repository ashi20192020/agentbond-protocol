use agentbond_types::{
    ConfigAccount, InitializeConfigPayload, ProtocolError, ProtocolEventKind, CONFIG_ACCOUNT_LEN,
    CONFIG_SEED,
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
use crate::processor::helpers::{next_account, now_ts, save_config};
use crate::token::require_legacy_token_program_id;

pub fn process(
    program_id: &Address,
    accounts: &[AccountView],
    payload: InitializeConfigPayload,
) -> ProgramResult {
    let mut accounts = accounts.iter();
    let admin = next_account(&mut accounts)?;
    let config_account = next_account(&mut accounts)?;
    let system_program = next_account(&mut accounts)?;

    require_signer(admin)?;
    require_system_program(system_program)?;
    require_legacy_token_program_id(&payload.token_program)?;

    if payload.min_provider_bond == 0 {
        return Err(fail(ProtocolError::InvalidAmount));
    }
    if payload.challenge_duration_seconds <= 0 {
        return Err(fail(ProtocolError::InvalidConfig));
    }
    if payload.allowed_mint == [0u8; 32] || payload.genesis_hash == [0u8; 32] {
        return Err(fail(ProtocolError::InvalidPubkey));
    }

    let (expected, bump) = Address::try_find_program_address(&[CONFIG_SEED], program_id)
        .ok_or_else(|| fail(ProtocolError::InvalidPda))?;
    if config_account.address() != &expected {
        return Err(fail(ProtocolError::InvalidPda));
    }
    require_uninitialized(config_account)?;

    let bump_seed = [bump];
    let seeds = [Seed::from(CONFIG_SEED), Seed::from(bump_seed.as_slice())];
    create_pda_account(
        admin,
        config_account,
        program_id,
        CONFIG_ACCOUNT_LEN,
        &seeds,
    )?;

    let config = ConfigAccount {
        bump,
        paused: false,
        admin: admin.address().to_bytes(),
        genesis_hash: payload.genesis_hash,
        allowed_mint: payload.allowed_mint,
        token_program: payload.token_program,
        mint_decimals: payload.mint_decimals,
        min_provider_bond: payload.min_provider_bond,
        challenge_duration_seconds: payload.challenge_duration_seconds,
    };
    save_config(config_account, &config)?;

    events::emit(
        ProtocolEventKind::ConfigInitialized,
        config_account.address(),
        admin.address(),
        0,
        now_ts()?,
    )
}
