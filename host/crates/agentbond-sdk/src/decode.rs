use agentbond_types::{
    CHALLENGE_ACCOUNT_LEN, CONFIG_ACCOUNT_LEN, ChallengeAccount, ConfigAccount, JOB_ACCOUNT_LEN,
    JobAccount, PROVIDER_ACCOUNT_LEN, PROVIDER_BOND_ACCOUNT_LEN, ProviderAccount,
    ProviderBondAccount,
};
use solana_pubkey::Pubkey;

use crate::address::{challenge_pda, config_pda, job_pda, provider_bond_pda, provider_pda};
use crate::error::SdkError;

fn require_owner(owner: &Pubkey, program_id: &Pubkey) -> Result<(), SdkError> {
    if owner != program_id {
        return Err(SdkError::WrongOwner);
    }
    Ok(())
}

fn require_address(actual: &Pubkey, expected: &Pubkey) -> Result<(), SdkError> {
    if actual != expected {
        return Err(SdkError::WrongAddress);
    }
    Ok(())
}

fn require_len(data: &[u8], expected: usize) -> Result<(), SdkError> {
    if data.len() != expected {
        return Err(SdkError::Decode(format!(
            "expected length {expected}, got {}",
            data.len()
        )));
    }
    Ok(())
}

pub fn decode_config(
    program_id: &Pubkey,
    address: &Pubkey,
    owner: &Pubkey,
    data: &[u8],
) -> Result<ConfigAccount, SdkError> {
    require_owner(owner, program_id)?;
    let expected = config_pda(program_id)?.address;
    require_address(address, &expected)?;
    require_len(data, CONFIG_ACCOUNT_LEN)?;
    ConfigAccount::decode(data).map_err(|e| SdkError::Decode(e.as_str().into()))
}

pub fn decode_provider(
    program_id: &Pubkey,
    address: &Pubkey,
    owner: &Pubkey,
    data: &[u8],
) -> Result<ProviderAccount, SdkError> {
    require_owner(owner, program_id)?;
    require_len(data, PROVIDER_ACCOUNT_LEN)?;
    let provider =
        ProviderAccount::decode(data).map_err(|e| SdkError::Decode(e.as_str().into()))?;
    let expected = provider_pda(program_id, &Pubkey::new_from_array(provider.authority))?.address;
    require_address(address, &expected)?;
    Ok(provider)
}

pub fn decode_provider_bond(
    program_id: &Pubkey,
    address: &Pubkey,
    owner: &Pubkey,
    data: &[u8],
) -> Result<ProviderBondAccount, SdkError> {
    require_owner(owner, program_id)?;
    require_len(data, PROVIDER_BOND_ACCOUNT_LEN)?;
    let bond =
        ProviderBondAccount::decode(data).map_err(|e| SdkError::Decode(e.as_str().into()))?;
    let expected = provider_bond_pda(
        program_id,
        &Pubkey::new_from_array(bond.provider),
        &Pubkey::new_from_array(bond.mint),
    )?
    .address;
    require_address(address, &expected)?;
    Ok(bond)
}

pub fn decode_job(
    program_id: &Pubkey,
    address: &Pubkey,
    owner: &Pubkey,
    data: &[u8],
) -> Result<JobAccount, SdkError> {
    require_owner(owner, program_id)?;
    require_len(data, JOB_ACCOUNT_LEN)?;
    let job = JobAccount::decode(data).map_err(|e| SdkError::Decode(e.as_str().into()))?;
    let expected = job_pda(
        program_id,
        &Pubkey::new_from_array(job.buyer),
        &Pubkey::new_from_array(job.provider),
        job.job_nonce,
    )?
    .address;
    require_address(address, &expected)?;
    Ok(job)
}

pub fn decode_challenge(
    program_id: &Pubkey,
    address: &Pubkey,
    owner: &Pubkey,
    data: &[u8],
) -> Result<ChallengeAccount, SdkError> {
    require_owner(owner, program_id)?;
    require_len(data, CHALLENGE_ACCOUNT_LEN)?;
    let challenge =
        ChallengeAccount::decode(data).map_err(|e| SdkError::Decode(e.as_str().into()))?;
    let expected = challenge_pda(program_id, &Pubkey::new_from_array(challenge.job))?.address;
    require_address(address, &expected)?;
    Ok(challenge)
}
