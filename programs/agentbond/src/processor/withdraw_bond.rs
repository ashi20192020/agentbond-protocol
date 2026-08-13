use agentbond_types::ProtocolError;
use agentbond_types::ProtocolEventKind;
use pinocchio::account::AccountView;
use pinocchio::address::Address;
use pinocchio::error::ProgramResult;

use crate::accounts::{bond_signer_seeds, require_signer, require_writable};
use crate::error::fail;
use crate::events;
use crate::processor::helpers::{load_validated_bond, next_account, now_ts, save_bond};
use crate::token::{
    require_legacy_token_program, transfer_checked, validate_ata, validate_mint,
    validate_token_account,
};

pub fn process(program_id: &Address, accounts: &[AccountView], amount: u64) -> ProgramResult {
    let mut accounts = accounts.iter();
    let authority = next_account(&mut accounts)?;
    let bond_account = next_account(&mut accounts)?;
    let bond_vault = next_account(&mut accounts)?;
    let authority_token = next_account(&mut accounts)?;
    let mint = next_account(&mut accounts)?;
    let token_program = next_account(&mut accounts)?;

    require_signer(authority)?;
    require_writable(bond_account)?;
    require_legacy_token_program(token_program)?;

    if amount == 0 {
        return Err(fail(ProtocolError::InvalidAmount));
    }

    let mut bond = load_validated_bond(program_id, bond_account)?;
    if bond.provider.as_ref() != authority.address().as_ref() {
        return Err(fail(ProtocolError::Unauthorized));
    }
    if token_program.address().as_ref() != bond.token_program {
        return Err(fail(ProtocolError::InvalidTokenProgram));
    }

    let unlocked = bond.unlocked().map_err(fail)?;
    if amount > unlocked {
        return Err(fail(ProtocolError::InsufficientBond));
    }

    // Decimals are not stored on bond; read from mint.
    let mint_state = pinocchio_token::state::Mint::from_account_view(mint)
        .map_err(|_| fail(ProtocolError::InvalidMint))?;
    let decimals = mint_state.decimals();
    validate_mint(mint, &bond.mint, decimals)?;
    validate_ata(bond_vault, bond_account.address(), &bond.mint)?;
    validate_token_account(authority_token, &bond.mint, authority.address())?;

    let bump = bond.bump;
    let seeds = bond_signer_seeds(&bond.provider, &bond.mint, &bump);
    transfer_checked(
        bond_vault,
        mint,
        authority_token,
        bond_account,
        amount,
        decimals,
        Some(&seeds),
    )?;

    bond.deposited = bond
        .deposited
        .checked_sub(amount)
        .ok_or_else(|| fail(ProtocolError::MathOverflow))?;
    if bond.locked > bond.deposited {
        return Err(fail(ProtocolError::InvalidAccountData));
    }
    save_bond(bond_account, &bond)?;

    events::emit(
        ProtocolEventKind::BondWithdrawn,
        bond_account.address(),
        authority.address(),
        amount,
        now_ts()?,
    )
}
