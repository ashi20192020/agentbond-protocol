use agentbond_types::ProtocolError;
use pinocchio::account::{AccountView, Ref};
use pinocchio::address::Address;
use pinocchio::cpi::{Seed, Signer};
use pinocchio::error::ProgramResult;
use pinocchio_token::instructions::{CloseAccount, TransferChecked};
use pinocchio_token::state::{Mint, TokenAccount};

use crate::accounts::{require_account_key, require_writable};
use crate::constants::{ASSOCIATED_TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID};
use crate::error::fail;

pub fn require_legacy_token_program(
    token_program: &AccountView,
) -> Result<(), pinocchio::error::ProgramError> {
    if token_program.address() == &TOKEN_2022_PROGRAM_ID {
        return Err(fail(ProtocolError::InvalidTokenProgram));
    }
    if token_program.address() != &TOKEN_PROGRAM_ID {
        return Err(fail(ProtocolError::InvalidTokenProgram));
    }
    Ok(())
}

pub fn require_legacy_token_program_id(
    token_program: &[u8; 32],
) -> Result<(), pinocchio::error::ProgramError> {
    if token_program == TOKEN_2022_PROGRAM_ID.as_ref() {
        return Err(fail(ProtocolError::InvalidTokenProgram));
    }
    if token_program != TOKEN_PROGRAM_ID.as_ref() {
        return Err(fail(ProtocolError::InvalidTokenProgram));
    }
    Ok(())
}

pub fn validate_mint(
    mint_account: &AccountView,
    expected_mint: &[u8; 32],
    expected_decimals: u8,
) -> Result<(), pinocchio::error::ProgramError> {
    require_account_key(
        mint_account,
        &Address::new_from_array(*expected_mint),
        ProtocolError::InvalidMint,
    )?;
    if !mint_account.owned_by(&TOKEN_PROGRAM_ID) {
        return Err(fail(ProtocolError::InvalidMint));
    }
    let mint =
        Mint::from_account_view(mint_account).map_err(|_| fail(ProtocolError::InvalidMint))?;
    if !mint.is_initialized() {
        return Err(fail(ProtocolError::InvalidMint));
    }
    if mint.decimals() != expected_decimals {
        return Err(fail(ProtocolError::InvalidMint));
    }
    Ok(())
}

pub fn parse_token_account(
    account: &AccountView,
) -> Result<Ref<'_, TokenAccount>, pinocchio::error::ProgramError> {
    if !account.owned_by(&TOKEN_PROGRAM_ID) {
        return Err(fail(ProtocolError::InvalidTokenAccountOwner));
    }
    TokenAccount::from_account_view(account).map_err(|_| fail(ProtocolError::InvalidTokenAccount))
}

pub fn validate_token_account(
    account: &AccountView,
    expected_mint: &[u8; 32],
    expected_owner: &Address,
) -> Result<u64, pinocchio::error::ProgramError> {
    let token = parse_token_account(account)?;
    if token.mint().as_ref() != expected_mint {
        return Err(fail(ProtocolError::InvalidTokenAccountMint));
    }
    if token.owner() != expected_owner {
        return Err(fail(ProtocolError::InvalidTokenAccountAuthority));
    }
    if token.is_frozen() {
        return Err(fail(ProtocolError::TokenAccountFrozen));
    }
    // Reject delegated authority as a substitute for expected ownership.
    if token.has_delegate() {
        return Err(fail(ProtocolError::InvalidTokenAccountAuthority));
    }
    Ok(token.amount())
}

pub fn associated_token_address(
    wallet: &Address,
    mint: &Address,
) -> Result<Address, pinocchio::error::ProgramError> {
    let seeds: &[&[u8]] = &[wallet.as_ref(), TOKEN_PROGRAM_ID.as_ref(), mint.as_ref()];
    Address::try_find_program_address(seeds, &ASSOCIATED_TOKEN_PROGRAM_ID)
        .map(|(address, _)| address)
        .ok_or_else(|| fail(ProtocolError::InvalidAssociatedTokenAccount))
}

pub fn validate_ata(
    account: &AccountView,
    wallet: &Address,
    mint: &[u8; 32],
) -> Result<u64, pinocchio::error::ProgramError> {
    let mint_address = Address::new_from_array(*mint);
    let expected = associated_token_address(wallet, &mint_address)?;
    require_account_key(
        account,
        &expected,
        ProtocolError::InvalidAssociatedTokenAccount,
    )?;
    validate_token_account(account, mint, wallet)
}

pub fn transfer_checked<'a>(
    from: &'a AccountView,
    mint: &'a AccountView,
    to: &'a AccountView,
    authority: &'a AccountView,
    amount: u64,
    decimals: u8,
    signer_seeds: Option<&[Seed]>,
) -> ProgramResult {
    require_writable(from)?;
    require_writable(to)?;
    let ix = TransferChecked {
        from,
        mint,
        to,
        authority,
        amount,
        decimals,
    };
    match signer_seeds {
        Some(seeds) => {
            let signer = Signer::from(seeds);
            ix.invoke_signed(&[signer])?
        }
        None => ix.invoke()?,
    }
    Ok(())
}

pub fn close_token_account<'a>(
    account: &'a AccountView,
    destination: &'a AccountView,
    authority: &'a AccountView,
    signer_seeds: &[Seed],
) -> ProgramResult {
    require_writable(account)?;
    require_writable(destination)?;
    let ix = CloseAccount {
        account,
        destination,
        authority,
    };
    let signer = Signer::from(signer_seeds);
    ix.invoke_signed(&[signer])?;
    Ok(())
}

pub fn return_surplus_to_buyer<'a>(
    escrow: &'a AccountView,
    mint: &'a AccountView,
    buyer_token: &'a AccountView,
    authority: &'a AccountView,
    principal: u64,
    decimals: u8,
    signer_seeds: &[Seed],
) -> ProgramResult {
    let balance = parse_token_account(escrow)?.amount();
    if balance < principal {
        return Err(fail(ProtocolError::EscrowUnexpectedBalance));
    }
    let surplus = balance
        .checked_sub(principal)
        .ok_or_else(|| fail(ProtocolError::MathOverflow))?;
    if surplus > 0 {
        transfer_checked(
            escrow,
            mint,
            buyer_token,
            authority,
            surplus,
            decimals,
            Some(signer_seeds),
        )?;
    }
    Ok(())
}

pub fn token_amount(account: &AccountView) -> Result<u64, pinocchio::error::ProgramError> {
    Ok(parse_token_account(account)?.amount())
}
