use agentbond_types::{
    bond_seed_parts, challenge_seed_parts, config_seed_parts, job_nonce_le_bytes, job_seed_parts,
    provider_seed_parts,
};
use pinocchio::address::Address;
use pinocchio::error::ProgramError;

/// Milestone 1 placeholder program id. Replace before any cluster deploy.
pub const ID: Address = Address::new_from_array([
    0x0a, 0x9e, 0xb1, 0x6d, 0x2c, 0x84, 0x3f, 0x51, 0x7a, 0xc2, 0x08, 0xd4, 0x6e, 0x35, 0x91, 0xbf,
    0x14, 0x67, 0xda, 0x2c, 0x58, 0x03, 0xee, 0x49, 0xb7, 0x1f, 0x85, 0x20, 0xcd, 0x63, 0xa4, 0x7e,
]);

pub fn config_address(program_id: &Address) -> Result<(Address, u8), ProgramError> {
    Address::try_find_program_address(&config_seed_parts(), program_id)
        .ok_or(ProgramError::InvalidSeeds)
}

pub fn provider_address(
    program_id: &Address,
    provider_authority: &[u8; 32],
) -> Result<(Address, u8), ProgramError> {
    Address::try_find_program_address(&provider_seed_parts(provider_authority), program_id)
        .ok_or(ProgramError::InvalidSeeds)
}

pub fn bond_address(
    program_id: &Address,
    provider_authority: &[u8; 32],
    mint: &[u8; 32],
) -> Result<(Address, u8), ProgramError> {
    Address::try_find_program_address(&bond_seed_parts(provider_authority, mint), program_id)
        .ok_or(ProgramError::InvalidSeeds)
}

pub fn job_address(
    program_id: &Address,
    buyer: &[u8; 32],
    provider: &[u8; 32],
    job_nonce: u64,
) -> Result<(Address, u8), ProgramError> {
    let nonce = job_nonce_le_bytes(job_nonce);
    Address::try_find_program_address(&job_seed_parts(buyer, provider, &nonce), program_id)
        .ok_or(ProgramError::InvalidSeeds)
}

pub fn challenge_address(
    program_id: &Address,
    job: &[u8; 32],
) -> Result<(Address, u8), ProgramError> {
    Address::try_find_program_address(&challenge_seed_parts(job), program_id)
        .ok_or(ProgramError::InvalidSeeds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_unique_pdas() {
        let (config, _) = config_address(&ID).expect("config");
        let authority = [1u8; 32];
        let mint = [2u8; 32];
        let buyer = [3u8; 32];
        let provider = [4u8; 32];
        let job_key = [5u8; 32];

        let (provider_pda, _) = provider_address(&ID, &authority).expect("provider");
        let (bond_pda, _) = bond_address(&ID, &authority, &mint).expect("bond");
        let (job_pda, _) = job_address(&ID, &buyer, &provider, 9).expect("job");
        let (challenge_pda, _) = challenge_address(&ID, &job_key).expect("challenge");

        let addresses = [config, provider_pda, bond_pda, job_pda, challenge_pda];
        for i in 0..addresses.len() {
            for j in (i + 1)..addresses.len() {
                assert_ne!(addresses[i], addresses[j]);
            }
        }
    }

    #[test]
    fn job_nonce_changes_address() {
        let buyer = [3u8; 32];
        let provider = [4u8; 32];
        let (a, _) = job_address(&ID, &buyer, &provider, 1).expect("job 1");
        let (b, _) = job_address(&ID, &buyer, &provider, 2).expect("job 2");
        assert_ne!(a, b);
    }
}
