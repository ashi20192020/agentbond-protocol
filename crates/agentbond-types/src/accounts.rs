use crate::error::ProtocolError;
use crate::state::JobState;

pub const ACCOUNT_LAYOUT_VERSION: u8 = 1;

pub const CONFIG_ACCOUNT_DISCRIMINATOR: u8 = 1;
pub const PROVIDER_ACCOUNT_DISCRIMINATOR: u8 = 2;
pub const PROVIDER_BOND_ACCOUNT_DISCRIMINATOR: u8 = 3;
pub const JOB_ACCOUNT_DISCRIMINATOR: u8 = 4;
pub const CHALLENGE_ACCOUNT_DISCRIMINATOR: u8 = 5;

pub const PROVIDER_STATUS_ACTIVE: u8 = 1;
pub const PROVIDER_STATUS_INACTIVE: u8 = 2;

pub const MAX_EXECUTION_KEYS: usize = 4;

pub const CONFIG_ACCOUNT_LEN: usize = 149;
pub const PROVIDER_ACCOUNT_LEN: usize = 165;
pub const PROVIDER_BOND_ACCOUNT_LEN: usize = 116;
pub const JOB_ACCOUNT_LEN: usize = 253;
pub const CHALLENGE_ACCOUNT_LEN: usize = 116;

fn read_bool(byte: u8) -> Result<bool, ProtocolError> {
    match byte {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(ProtocolError::InvalidBoolean),
    }
}

fn write_bool(value: bool) -> u8 {
    u8::from(value)
}

fn require_len(data: &[u8], expected: usize) -> Result<(), ProtocolError> {
    if data.len() != expected {
        Err(ProtocolError::InvalidAccountLength)
    } else {
        Ok(())
    }
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, ProtocolError> {
    let bytes: [u8; 8] = data
        .get(offset..offset + 8)
        .ok_or(ProtocolError::InvalidAccountData)?
        .try_into()
        .map_err(|_| ProtocolError::InvalidAccountData)?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_i64(data: &[u8], offset: usize) -> Result<i64, ProtocolError> {
    let bytes: [u8; 8] = data
        .get(offset..offset + 8)
        .ok_or(ProtocolError::InvalidAccountData)?
        .try_into()
        .map_err(|_| ProtocolError::InvalidAccountData)?;
    Ok(i64::from_le_bytes(bytes))
}

fn read_pubkey(data: &[u8], offset: usize) -> Result<[u8; 32], ProtocolError> {
    let mut out = [0u8; 32];
    out.copy_from_slice(
        data.get(offset..offset + 32)
            .ok_or(ProtocolError::InvalidAccountData)?,
    );
    Ok(out)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigAccount {
    pub bump: u8,
    pub paused: bool,
    pub admin: [u8; 32],
    pub genesis_hash: [u8; 32],
    pub allowed_mint: [u8; 32],
    pub token_program: [u8; 32],
    pub mint_decimals: u8,
    pub min_provider_bond: u64,
    pub challenge_duration_seconds: i64,
}

impl ConfigAccount {
    pub fn encode(&self) -> [u8; CONFIG_ACCOUNT_LEN] {
        let mut out = [0u8; CONFIG_ACCOUNT_LEN];
        out[0] = CONFIG_ACCOUNT_DISCRIMINATOR;
        out[1] = ACCOUNT_LAYOUT_VERSION;
        out[2] = self.bump;
        out[3] = write_bool(self.paused);
        out[4..36].copy_from_slice(&self.admin);
        out[36..68].copy_from_slice(&self.genesis_hash);
        out[68..100].copy_from_slice(&self.allowed_mint);
        out[100..132].copy_from_slice(&self.token_program);
        out[132] = self.mint_decimals;
        out[133..141].copy_from_slice(&self.min_provider_bond.to_le_bytes());
        out[141..149].copy_from_slice(&self.challenge_duration_seconds.to_le_bytes());
        out
    }

    pub fn decode(data: &[u8]) -> Result<Self, ProtocolError> {
        require_len(data, CONFIG_ACCOUNT_LEN)?;
        if data[0] != CONFIG_ACCOUNT_DISCRIMINATOR {
            return Err(ProtocolError::InvalidAccountDiscriminator);
        }
        if data[1] != ACCOUNT_LAYOUT_VERSION {
            return Err(ProtocolError::UnsupportedAccountVersion);
        }
        Ok(Self {
            bump: data[2],
            paused: read_bool(data[3])?,
            admin: read_pubkey(data, 4)?,
            genesis_hash: read_pubkey(data, 36)?,
            allowed_mint: read_pubkey(data, 68)?,
            token_program: read_pubkey(data, 100)?,
            mint_decimals: data[132],
            min_provider_bond: read_u64(data, 133)?,
            challenge_duration_seconds: read_i64(data, 141)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderAccount {
    pub bump: u8,
    pub status: u8,
    pub authority: [u8; 32],
    pub execution_key_count: u8,
    pub execution_keys: [[u8; 32]; MAX_EXECUTION_KEYS],
}

impl ProviderAccount {
    pub fn encode(&self) -> Result<[u8; PROVIDER_ACCOUNT_LEN], ProtocolError> {
        if self.status != PROVIDER_STATUS_ACTIVE && self.status != PROVIDER_STATUS_INACTIVE {
            return Err(ProtocolError::InvalidProviderStatus);
        }
        if usize::from(self.execution_key_count) > MAX_EXECUTION_KEYS {
            return Err(ProtocolError::InvalidAccountData);
        }

        let mut out = [0u8; PROVIDER_ACCOUNT_LEN];
        out[0] = PROVIDER_ACCOUNT_DISCRIMINATOR;
        out[1] = ACCOUNT_LAYOUT_VERSION;
        out[2] = self.bump;
        out[3] = self.status;
        out[4..36].copy_from_slice(&self.authority);
        out[36] = self.execution_key_count;
        for (index, key) in self.execution_keys.iter().enumerate() {
            let start = 37 + index * 32;
            out[start..start + 32].copy_from_slice(key);
        }
        Ok(out)
    }

    pub fn decode(data: &[u8]) -> Result<Self, ProtocolError> {
        require_len(data, PROVIDER_ACCOUNT_LEN)?;
        if data[0] != PROVIDER_ACCOUNT_DISCRIMINATOR {
            return Err(ProtocolError::InvalidAccountDiscriminator);
        }
        if data[1] != ACCOUNT_LAYOUT_VERSION {
            return Err(ProtocolError::UnsupportedAccountVersion);
        }
        if data[3] != PROVIDER_STATUS_ACTIVE && data[3] != PROVIDER_STATUS_INACTIVE {
            return Err(ProtocolError::InvalidProviderStatus);
        }
        if usize::from(data[36]) > MAX_EXECUTION_KEYS {
            return Err(ProtocolError::InvalidAccountData);
        }

        let mut execution_keys = [[0u8; 32]; MAX_EXECUTION_KEYS];
        for (index, key) in execution_keys.iter_mut().enumerate() {
            let start = 37 + index * 32;
            *key = read_pubkey(data, start)?;
        }

        Ok(Self {
            bump: data[2],
            status: data[3],
            authority: read_pubkey(data, 4)?,
            execution_key_count: data[36],
            execution_keys,
        })
    }

    pub fn contains_execution_key(&self, key: &[u8; 32]) -> bool {
        self.execution_keys
            .iter()
            .take(usize::from(self.execution_key_count))
            .any(|existing| existing == key)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderBondAccount {
    pub bump: u8,
    pub provider: [u8; 32],
    pub mint: [u8; 32],
    pub token_program: [u8; 32],
    pub deposited: u64,
    pub locked: u64,
}

impl ProviderBondAccount {
    pub fn unlocked(&self) -> Result<u64, ProtocolError> {
        self.deposited
            .checked_sub(self.locked)
            .ok_or(ProtocolError::MathOverflow)
    }

    pub fn encode(&self) -> Result<[u8; PROVIDER_BOND_ACCOUNT_LEN], ProtocolError> {
        if self.locked > self.deposited {
            return Err(ProtocolError::InvalidAccountData);
        }
        let mut out = [0u8; PROVIDER_BOND_ACCOUNT_LEN];
        out[0] = PROVIDER_BOND_ACCOUNT_DISCRIMINATOR;
        out[1] = ACCOUNT_LAYOUT_VERSION;
        out[2] = self.bump;
        out[3] = 0;
        out[4..36].copy_from_slice(&self.provider);
        out[36..68].copy_from_slice(&self.mint);
        out[68..100].copy_from_slice(&self.token_program);
        out[100..108].copy_from_slice(&self.deposited.to_le_bytes());
        out[108..116].copy_from_slice(&self.locked.to_le_bytes());
        Ok(out)
    }

    pub fn decode(data: &[u8]) -> Result<Self, ProtocolError> {
        require_len(data, PROVIDER_BOND_ACCOUNT_LEN)?;
        if data[0] != PROVIDER_BOND_ACCOUNT_DISCRIMINATOR {
            return Err(ProtocolError::InvalidAccountDiscriminator);
        }
        if data[1] != ACCOUNT_LAYOUT_VERSION {
            return Err(ProtocolError::UnsupportedAccountVersion);
        }
        let account = Self {
            bump: data[2],
            provider: read_pubkey(data, 4)?,
            mint: read_pubkey(data, 36)?,
            token_program: read_pubkey(data, 68)?,
            deposited: read_u64(data, 100)?,
            locked: read_u64(data, 108)?,
        };
        if account.locked > account.deposited {
            return Err(ProtocolError::InvalidAccountData);
        }
        Ok(account)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobAccount {
    pub bump: u8,
    pub state: JobState,
    pub buyer: [u8; 32],
    pub provider: [u8; 32],
    pub mint: [u8; 32],
    pub token_program: [u8; 32],
    pub amount: u64,
    pub job_nonce: u64,
    pub fund_deadline: i64,
    pub accept_deadline: i64,
    pub work_deadline: i64,
    pub auto_settle_deadline: i64,
    pub receipt_digest: [u8; 32],
    pub request_hash: [u8; 32],
    pub locked_bond: u64,
    pub mint_decimals: u8,
}

impl JobAccount {
    pub fn encode(&self) -> [u8; JOB_ACCOUNT_LEN] {
        let mut out = [0u8; JOB_ACCOUNT_LEN];
        out[0] = JOB_ACCOUNT_DISCRIMINATOR;
        out[1] = ACCOUNT_LAYOUT_VERSION;
        out[2] = self.bump;
        out[3] = self.state.as_u8();
        out[4..36].copy_from_slice(&self.buyer);
        out[36..68].copy_from_slice(&self.provider);
        out[68..100].copy_from_slice(&self.mint);
        out[100..132].copy_from_slice(&self.token_program);
        out[132..140].copy_from_slice(&self.amount.to_le_bytes());
        out[140..148].copy_from_slice(&self.job_nonce.to_le_bytes());
        out[148..156].copy_from_slice(&self.fund_deadline.to_le_bytes());
        out[156..164].copy_from_slice(&self.accept_deadline.to_le_bytes());
        out[164..172].copy_from_slice(&self.work_deadline.to_le_bytes());
        out[172..180].copy_from_slice(&self.auto_settle_deadline.to_le_bytes());
        out[180..212].copy_from_slice(&self.receipt_digest);
        out[212..244].copy_from_slice(&self.request_hash);
        out[244..252].copy_from_slice(&self.locked_bond.to_le_bytes());
        out[252] = self.mint_decimals;
        out
    }

    pub fn decode(data: &[u8]) -> Result<Self, ProtocolError> {
        require_len(data, JOB_ACCOUNT_LEN)?;
        if data[0] != JOB_ACCOUNT_DISCRIMINATOR {
            return Err(ProtocolError::InvalidAccountDiscriminator);
        }
        if data[1] != ACCOUNT_LAYOUT_VERSION {
            return Err(ProtocolError::UnsupportedAccountVersion);
        }
        Ok(Self {
            bump: data[2],
            state: JobState::from_u8(data[3])?,
            buyer: read_pubkey(data, 4)?,
            provider: read_pubkey(data, 36)?,
            mint: read_pubkey(data, 68)?,
            token_program: read_pubkey(data, 100)?,
            amount: read_u64(data, 132)?,
            job_nonce: read_u64(data, 140)?,
            fund_deadline: read_i64(data, 148)?,
            accept_deadline: read_i64(data, 156)?,
            work_deadline: read_i64(data, 164)?,
            auto_settle_deadline: read_i64(data, 172)?,
            receipt_digest: read_pubkey(data, 180)?,
            request_hash: read_pubkey(data, 212)?,
            locked_bond: read_u64(data, 244)?,
            mint_decimals: data[252],
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChallengeAccount {
    pub bump: u8,
    pub status: u8,
    pub job: [u8; 32],
    pub buyer: [u8; 32],
    pub reason_hash: [u8; 32],
    /// Must remain zero in Milestone 2.
    pub bond_amount: u64,
    pub deadline: i64,
}

impl ChallengeAccount {
    pub const STATUS_OPEN: u8 = 1;
    pub const STATUS_RESOLVED: u8 = 2;

    pub fn encode(&self) -> Result<[u8; CHALLENGE_ACCOUNT_LEN], ProtocolError> {
        if self.status != Self::STATUS_OPEN && self.status != Self::STATUS_RESOLVED {
            return Err(ProtocolError::InvalidChallengeStatus);
        }
        if self.bond_amount != 0 {
            return Err(ProtocolError::ChallengeBondMustBeZero);
        }
        let mut out = [0u8; CHALLENGE_ACCOUNT_LEN];
        out[0] = CHALLENGE_ACCOUNT_DISCRIMINATOR;
        out[1] = ACCOUNT_LAYOUT_VERSION;
        out[2] = self.bump;
        out[3] = self.status;
        out[4..36].copy_from_slice(&self.job);
        out[36..68].copy_from_slice(&self.buyer);
        out[68..100].copy_from_slice(&self.reason_hash);
        out[100..108].copy_from_slice(&self.bond_amount.to_le_bytes());
        out[108..116].copy_from_slice(&self.deadline.to_le_bytes());
        Ok(out)
    }

    pub fn decode(data: &[u8]) -> Result<Self, ProtocolError> {
        require_len(data, CHALLENGE_ACCOUNT_LEN)?;
        if data[0] != CHALLENGE_ACCOUNT_DISCRIMINATOR {
            return Err(ProtocolError::InvalidAccountDiscriminator);
        }
        if data[1] != ACCOUNT_LAYOUT_VERSION {
            return Err(ProtocolError::UnsupportedAccountVersion);
        }
        if data[3] != Self::STATUS_OPEN && data[3] != Self::STATUS_RESOLVED {
            return Err(ProtocolError::InvalidChallengeStatus);
        }
        let account = Self {
            bump: data[2],
            status: data[3],
            job: read_pubkey(data, 4)?,
            buyer: read_pubkey(data, 36)?,
            reason_hash: read_pubkey(data, 68)?,
            bond_amount: read_u64(data, 100)?,
            deadline: read_i64(data, 108)?,
        };
        if account.bond_amount != 0 {
            return Err(ProtocolError::ChallengeBondMustBeZero);
        }
        Ok(account)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sample_config() -> ConfigAccount {
        ConfigAccount {
            bump: 255,
            paused: false,
            admin: [9u8; 32],
            genesis_hash: [1u8; 32],
            allowed_mint: [2u8; 32],
            token_program: [3u8; 32],
            mint_decimals: 6,
            min_provider_bond: 1_000,
            challenge_duration_seconds: 3_600,
        }
    }

    fn sample_provider() -> ProviderAccount {
        let mut execution_keys = [[0u8; 32]; MAX_EXECUTION_KEYS];
        execution_keys[0] = [1u8; 32];
        execution_keys[1] = [2u8; 32];
        ProviderAccount {
            bump: 254,
            status: PROVIDER_STATUS_ACTIVE,
            authority: [3u8; 32],
            execution_key_count: 2,
            execution_keys,
        }
    }

    fn sample_bond() -> ProviderBondAccount {
        ProviderBondAccount {
            bump: 253,
            provider: [4u8; 32],
            mint: [5u8; 32],
            token_program: [6u8; 32],
            deposited: 1_000,
            locked: 250,
        }
    }

    fn sample_job() -> JobAccount {
        JobAccount {
            bump: 252,
            state: JobState::Funded,
            buyer: [7u8; 32],
            provider: [8u8; 32],
            mint: [9u8; 32],
            token_program: [10u8; 32],
            amount: 42,
            job_nonce: 7,
            fund_deadline: 100,
            accept_deadline: 200,
            work_deadline: 300,
            auto_settle_deadline: 400,
            receipt_digest: [11u8; 32],
            request_hash: [12u8; 32],
            locked_bond: 50,
            mint_decimals: 6,
        }
    }

    fn sample_challenge() -> ChallengeAccount {
        ChallengeAccount {
            bump: 251,
            status: ChallengeAccount::STATUS_OPEN,
            job: [12u8; 32],
            buyer: [13u8; 32],
            reason_hash: [14u8; 32],
            bond_amount: 0,
            deadline: 500,
        }
    }

    #[test]
    fn exact_encoded_sizes() {
        assert_eq!(sample_config().encode().len(), CONFIG_ACCOUNT_LEN);
        assert_eq!(
            sample_provider().encode().expect("encode").len(),
            PROVIDER_ACCOUNT_LEN
        );
        assert_eq!(
            sample_bond().encode().expect("encode").len(),
            PROVIDER_BOND_ACCOUNT_LEN
        );
        assert_eq!(sample_job().encode().len(), JOB_ACCOUNT_LEN);
        assert_eq!(
            sample_challenge().encode().expect("encode").len(),
            CHALLENGE_ACCOUNT_LEN
        );
        assert_eq!(CONFIG_ACCOUNT_LEN, 149);
        assert_eq!(PROVIDER_ACCOUNT_LEN, 165);
        assert_eq!(PROVIDER_BOND_ACCOUNT_LEN, 116);
        assert_eq!(JOB_ACCOUNT_LEN, 253);
        assert_eq!(CHALLENGE_ACCOUNT_LEN, 116);
    }

    #[test]
    fn round_trips() {
        assert_eq!(
            ConfigAccount::decode(&sample_config().encode()).expect("decode"),
            sample_config()
        );
        assert_eq!(
            ProviderAccount::decode(&sample_provider().encode().expect("encode")).expect("decode"),
            sample_provider()
        );
        assert_eq!(
            ProviderBondAccount::decode(&sample_bond().encode().expect("encode")).expect("decode"),
            sample_bond()
        );
        assert_eq!(
            JobAccount::decode(&sample_job().encode()).expect("decode"),
            sample_job()
        );
        assert_eq!(
            ChallengeAccount::decode(&sample_challenge().encode().expect("encode"))
                .expect("decode"),
            sample_challenge()
        );
    }

    #[test]
    fn wrong_discriminator() {
        let mut data = sample_config().encode();
        data[0] = 99;
        assert_eq!(
            ConfigAccount::decode(&data),
            Err(ProtocolError::InvalidAccountDiscriminator)
        );
    }

    #[test]
    fn unsupported_layout_version() {
        let mut data = sample_job().encode();
        data[1] = 2;
        assert_eq!(
            JobAccount::decode(&data),
            Err(ProtocolError::UnsupportedAccountVersion)
        );
    }

    #[test]
    fn truncated_and_oversized_input() {
        let encoded = sample_config().encode();
        assert_eq!(
            ConfigAccount::decode(&encoded[..CONFIG_ACCOUNT_LEN - 1]),
            Err(ProtocolError::InvalidAccountLength)
        );
        let mut oversized = encoded.to_vec();
        oversized.push(0);
        assert_eq!(
            ConfigAccount::decode(&oversized),
            Err(ProtocolError::InvalidAccountLength)
        );
    }

    #[test]
    fn malformed_state_value() {
        let mut data = sample_job().encode();
        data[3] = 99;
        assert_eq!(
            JobAccount::decode(&data),
            Err(ProtocolError::InvalidJobState)
        );
    }

    #[test]
    fn challenge_nonzero_bond_rejected() {
        let mut challenge = sample_challenge();
        challenge.bond_amount = 1;
        assert_eq!(
            challenge.encode(),
            Err(ProtocolError::ChallengeBondMustBeZero)
        );
    }

    #[test]
    fn bond_locked_above_deposited_rejected() {
        let mut bond = sample_bond();
        bond.locked = bond.deposited + 1;
        assert_eq!(bond.encode(), Err(ProtocolError::InvalidAccountData));
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic(data in prop::collection::vec(any::<u8>(), 0..300)) {
            let _ = ConfigAccount::decode(&data);
            let _ = ProviderAccount::decode(&data);
            let _ = ProviderBondAccount::decode(&data);
            let _ = JobAccount::decode(&data);
            let _ = ChallengeAccount::decode(&data);
        }
    }
}
