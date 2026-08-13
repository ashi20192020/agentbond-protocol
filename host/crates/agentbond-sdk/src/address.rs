use agentbond_types::{
    bond_seed_parts, challenge_seed_parts, config_seed_parts, job_nonce_le_bytes, job_seed_parts,
    provider_seed_parts,
};
use solana_pubkey::Pubkey;
use spl_associated_token_account_client::address::get_associated_token_address_with_program_id;
use spl_token::ID as TOKEN_PROGRAM_ID;

use crate::error::SdkError;

/// Milestone 2/3 program id (matches onchain `programs/agentbond` placeholder).
pub const PROGRAM_ID_BYTES: [u8; 32] = [
    0x0a, 0x9e, 0xb1, 0x6d, 0x2c, 0x84, 0x3f, 0x51, 0x7a, 0xc2, 0x08, 0xd4, 0x6e, 0x35, 0x91, 0xbf,
    0x14, 0x67, 0xda, 0x2c, 0x58, 0x03, 0xee, 0x49, 0xb7, 0x1f, 0x85, 0x20, 0xcd, 0x63, 0xa4, 0x7e,
];

pub fn program_id() -> Pubkey {
    Pubkey::new_from_array(PROGRAM_ID_BYTES)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pda {
    pub address: Pubkey,
    pub bump: u8,
}

fn find_pda(seeds: &[&[u8]], program_id: &Pubkey) -> Result<Pda, SdkError> {
    Pubkey::try_find_program_address(seeds, program_id)
        .map(|(address, bump)| Pda { address, bump })
        .ok_or_else(|| SdkError::InvalidInput("pda derivation failed".into()))
}

pub fn config_pda(program_id: &Pubkey) -> Result<Pda, SdkError> {
    let seeds = config_seed_parts();
    find_pda(&seeds, program_id)
}

pub fn provider_pda(program_id: &Pubkey, authority: &Pubkey) -> Result<Pda, SdkError> {
    let auth = authority.to_bytes();
    let seeds = provider_seed_parts(&auth);
    find_pda(&seeds, program_id)
}

pub fn provider_bond_pda(
    program_id: &Pubkey,
    authority: &Pubkey,
    mint: &Pubkey,
) -> Result<Pda, SdkError> {
    let auth = authority.to_bytes();
    let mint_bytes = mint.to_bytes();
    let seeds = bond_seed_parts(&auth, &mint_bytes);
    find_pda(&seeds, program_id)
}

pub fn job_pda(
    program_id: &Pubkey,
    buyer: &Pubkey,
    provider: &Pubkey,
    nonce: u64,
) -> Result<Pda, SdkError> {
    let buyer_b = buyer.to_bytes();
    let provider_b = provider.to_bytes();
    let nonce_b = job_nonce_le_bytes(nonce);
    let seeds = job_seed_parts(&buyer_b, &provider_b, &nonce_b);
    find_pda(&seeds, program_id)
}

pub fn challenge_pda(program_id: &Pubkey, job: &Pubkey) -> Result<Pda, SdkError> {
    let job_b = job.to_bytes();
    let seeds = challenge_seed_parts(&job_b);
    find_pda(&seeds, program_id)
}

pub fn bond_vault_ata(bond_pda: &Pubkey, mint: &Pubkey) -> Pubkey {
    get_associated_token_address_with_program_id(bond_pda, mint, &TOKEN_PROGRAM_ID)
}

pub fn job_escrow_ata(job: &Pubkey, mint: &Pubkey) -> Pubkey {
    get_associated_token_address_with_program_id(job, mint, &TOKEN_PROGRAM_ID)
}

pub fn user_settlement_ata(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    get_associated_token_address_with_program_id(owner, mint, &TOKEN_PROGRAM_ID)
}

pub fn parse_pubkey(s: &str) -> Result<Pubkey, SdkError> {
    s.parse::<Pubkey>()
        .map_err(|_| SdkError::InvalidPubkey(s.to_string()))
}
