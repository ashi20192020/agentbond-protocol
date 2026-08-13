pub const CONFIG_SEED: &[u8] = b"config";
pub const PROVIDER_SEED: &[u8] = b"provider";
pub const BOND_SEED: &[u8] = b"bond";
pub const JOB_SEED: &[u8] = b"job";
pub const CHALLENGE_SEED: &[u8] = b"challenge";

pub fn job_nonce_le_bytes(job_nonce: u64) -> [u8; 8] {
    job_nonce.to_le_bytes()
}

pub fn config_seed_parts() -> [&'static [u8]; 1] {
    [CONFIG_SEED]
}

pub fn provider_seed_parts(provider_authority: &[u8; 32]) -> [&[u8]; 2] {
    [PROVIDER_SEED, provider_authority.as_slice()]
}

pub fn bond_seed_parts<'a>(provider_authority: &'a [u8; 32], mint: &'a [u8; 32]) -> [&'a [u8]; 3] {
    [BOND_SEED, provider_authority.as_slice(), mint.as_slice()]
}

pub fn job_seed_parts<'a>(
    buyer: &'a [u8; 32],
    provider: &'a [u8; 32],
    job_nonce_le: &'a [u8; 8],
) -> [&'a [u8]; 4] {
    [
        JOB_SEED,
        buyer.as_slice(),
        provider.as_slice(),
        job_nonce_le.as_slice(),
    ]
}

pub fn challenge_seed_parts(job: &[u8; 32]) -> [&[u8]; 2] {
    [CHALLENGE_SEED, job.as_slice()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_constants() {
        assert_eq!(CONFIG_SEED, b"config");
        assert_eq!(PROVIDER_SEED, b"provider");
        assert_eq!(BOND_SEED, b"bond");
        assert_eq!(JOB_SEED, b"job");
        assert_eq!(CHALLENGE_SEED, b"challenge");
    }

    #[test]
    fn job_nonce_little_endian() {
        assert_eq!(job_nonce_le_bytes(1), 1u64.to_le_bytes());
        assert_eq!(job_nonce_le_bytes(u64::MAX), u64::MAX.to_le_bytes());
    }

    #[test]
    fn seed_part_lengths() {
        let authority = [9u8; 32];
        let mint = [8u8; 32];
        let buyer = [7u8; 32];
        let provider = [6u8; 32];
        let nonce = job_nonce_le_bytes(42);
        let job = [5u8; 32];

        assert_eq!(config_seed_parts().len(), 1);
        assert_eq!(provider_seed_parts(&authority).len(), 2);
        assert_eq!(bond_seed_parts(&authority, &mint).len(), 3);
        assert_eq!(job_seed_parts(&buyer, &provider, &nonce).len(), 4);
        assert_eq!(challenge_seed_parts(&job).len(), 2);
    }
}
