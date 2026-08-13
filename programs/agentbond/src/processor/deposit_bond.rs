use agentbond_types::{
    ProtocolError, ProtocolEventKind, ProviderBondAccount, BOND_SEED, PROVIDER_BOND_ACCOUNT_LEN,
    PROVIDER_STATUS_ACTIVE,
};
use pinocchio::account::AccountView;
use pinocchio::address::Address;
use pinocchio::cpi::Seed;
use pinocchio::error::ProgramResult;

use crate::accounts::{
    address_from_bytes, create_pda_account, require_signer, require_system_program,
    require_uninitialized, require_writable,
};
use crate::constants::SYSTEM_PROGRAM_ID;
use crate::error::fail;
use crate::events;
use crate::processor::helpers::{
    load_validated_bond, load_validated_config_readonly, load_validated_provider, next_account,
    now_ts, require_not_paused, save_bond,
};
use crate::token::{
    associated_token_address, require_legacy_token_program, transfer_checked, validate_ata,
    validate_mint, validate_token_account,
};

pub fn process(program_id: &Address, accounts: &[AccountView], amount: u64) -> ProgramResult {
    let mut accounts = accounts.iter();
    let authority = next_account(&mut accounts)?;
    let config_account = next_account(&mut accounts)?;
    let provider_account = next_account(&mut accounts)?;
    let bond_account = next_account(&mut accounts)?;
    let bond_vault = next_account(&mut accounts)?;
    let authority_token = next_account(&mut accounts)?;
    let mint = next_account(&mut accounts)?;
    let token_program = next_account(&mut accounts)?;
    let system_program = next_account(&mut accounts)?;

    require_signer(authority)?;
    require_system_program(system_program)?;
    require_legacy_token_program(token_program)?;

    if amount == 0 {
        return Err(fail(ProtocolError::InvalidAmount));
    }

    let config = load_validated_config_readonly(program_id, config_account)?;
    require_not_paused(&config)?;
    if token_program.address().as_ref() != config.token_program {
        return Err(fail(ProtocolError::InvalidTokenProgram));
    }

    let provider = load_validated_provider(program_id, provider_account)?;
    if provider.authority.as_ref() != authority.address().as_ref() {
        return Err(fail(ProtocolError::Unauthorized));
    }
    if provider.status != PROVIDER_STATUS_ACTIVE {
        return Err(fail(ProtocolError::ProviderInactive));
    }

    validate_mint(mint, &config.allowed_mint, config.mint_decimals)?;
    validate_token_account(authority_token, &config.allowed_mint, authority.address())?;

    let authority_bytes = authority.address().to_bytes();
    let (expected_bond, bump) = Address::try_find_program_address(
        &[
            BOND_SEED,
            authority_bytes.as_slice(),
            config.allowed_mint.as_slice(),
        ],
        program_id,
    )
    .ok_or_else(|| fail(ProtocolError::InvalidPda))?;
    if bond_account.address() != &expected_bond {
        return Err(fail(ProtocolError::InvalidPda));
    }

    let bond = if bond_account.owned_by(&SYSTEM_PROGRAM_ID) && bond_account.is_data_empty() {
        require_uninitialized(bond_account)?;
        let bump_seed = [bump];
        let seeds = [
            Seed::from(BOND_SEED),
            Seed::from(authority_bytes.as_slice()),
            Seed::from(config.allowed_mint.as_slice()),
            Seed::from(bump_seed.as_slice()),
        ];
        create_pda_account(
            authority,
            bond_account,
            program_id,
            PROVIDER_BOND_ACCOUNT_LEN,
            &seeds,
        )?;
        ProviderBondAccount {
            bump,
            provider: authority_bytes,
            mint: config.allowed_mint,
            token_program: config.token_program,
            deposited: 0,
            locked: 0,
        }
    } else {
        require_writable(bond_account)?;
        let existing = load_validated_bond(program_id, bond_account)?;
        if existing.provider != authority_bytes || existing.mint != config.allowed_mint {
            return Err(fail(ProtocolError::InvalidPda));
        }
        existing
    };

    let vault_owner = bond_account.address();
    validate_ata(bond_vault, vault_owner, &config.allowed_mint)?;
    let expected_vault =
        associated_token_address(vault_owner, &address_from_bytes(&config.allowed_mint))?;
    if bond_vault.address() != &expected_vault {
        return Err(fail(ProtocolError::InvalidAssociatedTokenAccount));
    }

    transfer_checked(
        authority_token,
        mint,
        bond_vault,
        authority,
        amount,
        config.mint_decimals,
        None,
    )?;

    let mut bond = bond;
    bond.deposited = bond
        .deposited
        .checked_add(amount)
        .ok_or_else(|| fail(ProtocolError::MathOverflow))?;
    save_bond(bond_account, &bond)?;

    events::emit(
        ProtocolEventKind::BondDeposited,
        bond_account.address(),
        authority.address(),
        amount,
        now_ts()?,
    )
}
