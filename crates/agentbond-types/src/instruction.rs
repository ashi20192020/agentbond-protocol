use crate::error::ProtocolError;
use crate::receipt::{AgentBondWorkReceiptV1, RECEIPT_ENCODED_LEN};

pub const INSTRUCTION_DISCRIMINATOR_LEN: usize = 1;

pub const INITIALIZE_CONFIG_PAYLOAD_LEN: usize = 113;
pub const SET_PAUSED_PAYLOAD_LEN: usize = 1;
pub const ADD_EXECUTION_KEY_PAYLOAD_LEN: usize = 32;
pub const REVOKE_EXECUTION_KEY_PAYLOAD_LEN: usize = 32;
pub const DEPOSIT_BOND_PAYLOAD_LEN: usize = 8;
pub const WITHDRAW_BOND_PAYLOAD_LEN: usize = 8;
pub const CREATE_JOB_PAYLOAD_LEN: usize = 80;
pub const SUBMIT_RECEIPT_PAYLOAD_LEN: usize = RECEIPT_ENCODED_LEN;
pub const CHALLENGE_WORK_PAYLOAD_LEN: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum InstructionKind {
    InitializeConfig = 0,
    RegisterProvider = 1,
    AddExecutionKey = 2,
    RevokeExecutionKey = 3,
    DepositBond = 4,
    WithdrawBond = 5,
    CreateJob = 6,
    FundJob = 7,
    AcceptJob = 8,
    SubmitReceipt = 9,
    AcceptWork = 10,
    ChallengeWork = 11,
    ResolveTimeoutSettle = 12,
    ResolveTimeoutRefund = 13,
    ExpireUnfunded = 14,
    ExpireUnaccepted = 15,
    SlashBond = 16,
    CloseJob = 17,
    SetPaused = 18,
}

impl InstructionKind {
    pub const fn from_u8(value: u8) -> Result<Self, ProtocolError> {
        match value {
            0 => Ok(Self::InitializeConfig),
            1 => Ok(Self::RegisterProvider),
            2 => Ok(Self::AddExecutionKey),
            3 => Ok(Self::RevokeExecutionKey),
            4 => Ok(Self::DepositBond),
            5 => Ok(Self::WithdrawBond),
            6 => Ok(Self::CreateJob),
            7 => Ok(Self::FundJob),
            8 => Ok(Self::AcceptJob),
            9 => Ok(Self::SubmitReceipt),
            10 => Ok(Self::AcceptWork),
            11 => Ok(Self::ChallengeWork),
            12 => Ok(Self::ResolveTimeoutSettle),
            13 => Ok(Self::ResolveTimeoutRefund),
            14 => Ok(Self::ExpireUnfunded),
            15 => Ok(Self::ExpireUnaccepted),
            16 => Ok(Self::SlashBond),
            17 => Ok(Self::CloseJob),
            18 => Ok(Self::SetPaused),
            _ => Err(ProtocolError::UnknownInstruction),
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn expected_payload_len(self) -> usize {
        match self {
            Self::InitializeConfig => INITIALIZE_CONFIG_PAYLOAD_LEN,
            Self::RegisterProvider => 0,
            Self::AddExecutionKey => ADD_EXECUTION_KEY_PAYLOAD_LEN,
            Self::RevokeExecutionKey => REVOKE_EXECUTION_KEY_PAYLOAD_LEN,
            Self::DepositBond => DEPOSIT_BOND_PAYLOAD_LEN,
            Self::WithdrawBond => WITHDRAW_BOND_PAYLOAD_LEN,
            Self::CreateJob => CREATE_JOB_PAYLOAD_LEN,
            Self::FundJob => 0,
            Self::AcceptJob => 0,
            Self::SubmitReceipt => SUBMIT_RECEIPT_PAYLOAD_LEN,
            Self::AcceptWork => 0,
            Self::ChallengeWork => CHALLENGE_WORK_PAYLOAD_LEN,
            Self::ResolveTimeoutSettle => 0,
            Self::ResolveTimeoutRefund => 0,
            Self::ExpireUnfunded => 0,
            Self::ExpireUnaccepted => 0,
            Self::SlashBond => 0,
            Self::CloseJob => 0,
            Self::SetPaused => SET_PAUSED_PAYLOAD_LEN,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InitializeConfigPayload {
    pub genesis_hash: [u8; 32],
    pub allowed_mint: [u8; 32],
    pub token_program: [u8; 32],
    pub mint_decimals: u8,
    pub min_provider_bond: u64,
    pub challenge_duration_seconds: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreateJobPayload {
    pub job_nonce: u64,
    pub amount: u64,
    pub request_hash: [u8; 32],
    pub fund_deadline: i64,
    pub accept_deadline: i64,
    pub work_deadline: i64,
    pub auto_settle_deadline: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Instruction {
    InitializeConfig(InitializeConfigPayload),
    RegisterProvider,
    AddExecutionKey { key: [u8; 32] },
    RevokeExecutionKey { key: [u8; 32] },
    DepositBond { amount: u64 },
    WithdrawBond { amount: u64 },
    CreateJob(CreateJobPayload),
    FundJob,
    AcceptJob,
    SubmitReceipt(AgentBondWorkReceiptV1),
    AcceptWork,
    ChallengeWork { reason_hash: [u8; 32] },
    ResolveTimeoutSettle,
    ResolveTimeoutRefund,
    ExpireUnfunded,
    ExpireUnaccepted,
    SlashBond,
    CloseJob,
    SetPaused { paused: bool },
}

impl Instruction {
    pub fn kind(&self) -> InstructionKind {
        match self {
            Self::InitializeConfig(_) => InstructionKind::InitializeConfig,
            Self::RegisterProvider => InstructionKind::RegisterProvider,
            Self::AddExecutionKey { .. } => InstructionKind::AddExecutionKey,
            Self::RevokeExecutionKey { .. } => InstructionKind::RevokeExecutionKey,
            Self::DepositBond { .. } => InstructionKind::DepositBond,
            Self::WithdrawBond { .. } => InstructionKind::WithdrawBond,
            Self::CreateJob(_) => InstructionKind::CreateJob,
            Self::FundJob => InstructionKind::FundJob,
            Self::AcceptJob => InstructionKind::AcceptJob,
            Self::SubmitReceipt(_) => InstructionKind::SubmitReceipt,
            Self::AcceptWork => InstructionKind::AcceptWork,
            Self::ChallengeWork { .. } => InstructionKind::ChallengeWork,
            Self::ResolveTimeoutSettle => InstructionKind::ResolveTimeoutSettle,
            Self::ResolveTimeoutRefund => InstructionKind::ResolveTimeoutRefund,
            Self::ExpireUnfunded => InstructionKind::ExpireUnfunded,
            Self::ExpireUnaccepted => InstructionKind::ExpireUnaccepted,
            Self::SlashBond => InstructionKind::SlashBond,
            Self::CloseJob => InstructionKind::CloseJob,
            Self::SetPaused { .. } => InstructionKind::SetPaused,
        }
    }
}

fn read_pubkey(data: &[u8], offset: usize) -> Result<[u8; 32], ProtocolError> {
    let mut out = [0u8; 32];
    out.copy_from_slice(
        data.get(offset..offset + 32)
            .ok_or(ProtocolError::InvalidInstructionData)?,
    );
    Ok(out)
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, ProtocolError> {
    let bytes: [u8; 8] = data
        .get(offset..offset + 8)
        .ok_or(ProtocolError::InvalidInstructionData)?
        .try_into()
        .map_err(|_| ProtocolError::InvalidInstructionData)?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_i64(data: &[u8], offset: usize) -> Result<i64, ProtocolError> {
    let bytes: [u8; 8] = data
        .get(offset..offset + 8)
        .ok_or(ProtocolError::InvalidInstructionData)?
        .try_into()
        .map_err(|_| ProtocolError::InvalidInstructionData)?;
    Ok(i64::from_le_bytes(bytes))
}

fn read_bool(byte: u8) -> Result<bool, ProtocolError> {
    match byte {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(ProtocolError::InvalidBoolean),
    }
}

pub fn parse_instruction(data: &[u8]) -> Result<Instruction, ProtocolError> {
    if data.is_empty() {
        return Err(ProtocolError::EmptyInstructionData);
    }

    let kind = InstructionKind::from_u8(data[0])?;
    let payload = &data[INSTRUCTION_DISCRIMINATOR_LEN..];
    if payload.len() != kind.expected_payload_len() {
        return Err(ProtocolError::InvalidInstructionLength);
    }

    match kind {
        InstructionKind::InitializeConfig => {
            Ok(Instruction::InitializeConfig(InitializeConfigPayload {
                genesis_hash: read_pubkey(payload, 0)?,
                allowed_mint: read_pubkey(payload, 32)?,
                token_program: read_pubkey(payload, 64)?,
                mint_decimals: payload[96],
                min_provider_bond: read_u64(payload, 97)?,
                challenge_duration_seconds: read_i64(payload, 105)?,
            }))
        }
        InstructionKind::RegisterProvider => Ok(Instruction::RegisterProvider),
        InstructionKind::AddExecutionKey => Ok(Instruction::AddExecutionKey {
            key: read_pubkey(payload, 0)?,
        }),
        InstructionKind::RevokeExecutionKey => Ok(Instruction::RevokeExecutionKey {
            key: read_pubkey(payload, 0)?,
        }),
        InstructionKind::DepositBond => Ok(Instruction::DepositBond {
            amount: read_u64(payload, 0)?,
        }),
        InstructionKind::WithdrawBond => Ok(Instruction::WithdrawBond {
            amount: read_u64(payload, 0)?,
        }),
        InstructionKind::CreateJob => Ok(Instruction::CreateJob(CreateJobPayload {
            job_nonce: read_u64(payload, 0)?,
            amount: read_u64(payload, 8)?,
            request_hash: read_pubkey(payload, 16)?,
            fund_deadline: read_i64(payload, 48)?,
            accept_deadline: read_i64(payload, 56)?,
            work_deadline: read_i64(payload, 64)?,
            auto_settle_deadline: read_i64(payload, 72)?,
        })),
        InstructionKind::FundJob => Ok(Instruction::FundJob),
        InstructionKind::AcceptJob => Ok(Instruction::AcceptJob),
        InstructionKind::SubmitReceipt => {
            let receipt = AgentBondWorkReceiptV1::decode(payload)?;
            Ok(Instruction::SubmitReceipt(receipt))
        }
        InstructionKind::AcceptWork => Ok(Instruction::AcceptWork),
        InstructionKind::ChallengeWork => Ok(Instruction::ChallengeWork {
            reason_hash: read_pubkey(payload, 0)?,
        }),
        InstructionKind::ResolveTimeoutSettle => Ok(Instruction::ResolveTimeoutSettle),
        InstructionKind::ResolveTimeoutRefund => Ok(Instruction::ResolveTimeoutRefund),
        InstructionKind::ExpireUnfunded => Ok(Instruction::ExpireUnfunded),
        InstructionKind::ExpireUnaccepted => Ok(Instruction::ExpireUnaccepted),
        InstructionKind::SlashBond => Ok(Instruction::SlashBond),
        InstructionKind::CloseJob => Ok(Instruction::CloseJob),
        InstructionKind::SetPaused => Ok(Instruction::SetPaused {
            paused: read_bool(payload[0])?,
        }),
    }
}

pub fn encode_initialize_config(
    payload: &InitializeConfigPayload,
) -> [u8; 1 + INITIALIZE_CONFIG_PAYLOAD_LEN] {
    let mut out = [0u8; 1 + INITIALIZE_CONFIG_PAYLOAD_LEN];
    out[0] = InstructionKind::InitializeConfig.as_u8();
    out[1..33].copy_from_slice(&payload.genesis_hash);
    out[33..65].copy_from_slice(&payload.allowed_mint);
    out[65..97].copy_from_slice(&payload.token_program);
    out[97] = payload.mint_decimals;
    out[98..106].copy_from_slice(&payload.min_provider_bond.to_le_bytes());
    out[106..114].copy_from_slice(&payload.challenge_duration_seconds.to_le_bytes());
    out
}

pub fn encode_set_paused(paused: bool) -> [u8; 2] {
    [InstructionKind::SetPaused.as_u8(), u8::from(paused)]
}

pub fn encode_add_execution_key(key: &[u8; 32]) -> [u8; 33] {
    let mut out = [0u8; 33];
    out[0] = InstructionKind::AddExecutionKey.as_u8();
    out[1..].copy_from_slice(key);
    out
}

pub fn encode_revoke_execution_key(key: &[u8; 32]) -> [u8; 33] {
    let mut out = [0u8; 33];
    out[0] = InstructionKind::RevokeExecutionKey.as_u8();
    out[1..].copy_from_slice(key);
    out
}

pub fn encode_deposit_bond(amount: u64) -> [u8; 9] {
    let mut out = [0u8; 9];
    out[0] = InstructionKind::DepositBond.as_u8();
    out[1..].copy_from_slice(&amount.to_le_bytes());
    out
}

pub fn encode_withdraw_bond(amount: u64) -> [u8; 9] {
    let mut out = [0u8; 9];
    out[0] = InstructionKind::WithdrawBond.as_u8();
    out[1..].copy_from_slice(&amount.to_le_bytes());
    out
}

pub fn encode_create_job(payload: &CreateJobPayload) -> [u8; 1 + CREATE_JOB_PAYLOAD_LEN] {
    let mut out = [0u8; 1 + CREATE_JOB_PAYLOAD_LEN];
    out[0] = InstructionKind::CreateJob.as_u8();
    out[1..9].copy_from_slice(&payload.job_nonce.to_le_bytes());
    out[9..17].copy_from_slice(&payload.amount.to_le_bytes());
    out[17..49].copy_from_slice(&payload.request_hash);
    out[49..57].copy_from_slice(&payload.fund_deadline.to_le_bytes());
    out[57..65].copy_from_slice(&payload.accept_deadline.to_le_bytes());
    out[65..73].copy_from_slice(&payload.work_deadline.to_le_bytes());
    out[73..81].copy_from_slice(&payload.auto_settle_deadline.to_le_bytes());
    out
}

pub fn encode_submit_receipt(
    receipt: &AgentBondWorkReceiptV1,
) -> Result<[u8; 1 + SUBMIT_RECEIPT_PAYLOAD_LEN], ProtocolError> {
    let mut out = [0u8; 1 + SUBMIT_RECEIPT_PAYLOAD_LEN];
    out[0] = InstructionKind::SubmitReceipt.as_u8();
    out[1..].copy_from_slice(&receipt.encode()?);
    Ok(out)
}

pub fn encode_challenge_work(reason_hash: &[u8; 32]) -> [u8; 33] {
    let mut out = [0u8; 33];
    out[0] = InstructionKind::ChallengeWork.as_u8();
    out[1..].copy_from_slice(reason_hash);
    out
}

pub fn encode_empty(kind: InstructionKind) -> Result<[u8; 1], ProtocolError> {
    if kind.expected_payload_len() != 0 {
        return Err(ProtocolError::InvalidInstructionData);
    }
    Ok([kind.as_u8()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn known_discriminator_parsing() {
        for value in 0u8..=18 {
            let kind = InstructionKind::from_u8(value).expect("known kind");
            assert_eq!(kind.as_u8(), value);
        }
    }

    #[test]
    fn empty_data_rejected() {
        assert_eq!(
            parse_instruction(&[]),
            Err(ProtocolError::EmptyInstructionData)
        );
    }

    #[test]
    fn unknown_discriminator_rejected() {
        assert_eq!(
            parse_instruction(&[19]),
            Err(ProtocolError::UnknownInstruction)
        );
    }

    #[test]
    fn incorrect_payload_length_rejected() {
        assert_eq!(
            parse_instruction(&[InstructionKind::CreateJob.as_u8(), 0x00]),
            Err(ProtocolError::InvalidInstructionLength)
        );
    }

    #[test]
    fn initialize_config_golden_round_trip() {
        let payload = InitializeConfigPayload {
            genesis_hash: [1u8; 32],
            allowed_mint: [2u8; 32],
            token_program: [3u8; 32],
            mint_decimals: 6,
            min_provider_bond: 1_000,
            challenge_duration_seconds: 3_600,
        };
        let encoded = encode_initialize_config(&payload);
        assert_eq!(encoded.len(), 114);
        match parse_instruction(&encoded).expect("parse") {
            Instruction::InitializeConfig(decoded) => assert_eq!(decoded, payload),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn create_job_golden_round_trip() {
        let payload = CreateJobPayload {
            job_nonce: 9,
            amount: 500,
            request_hash: [4u8; 32],
            fund_deadline: 10,
            accept_deadline: 20,
            work_deadline: 30,
            auto_settle_deadline: 40,
        };
        let encoded = encode_create_job(&payload);
        assert_eq!(encoded.len(), 81);
        match parse_instruction(&encoded).expect("parse") {
            Instruction::CreateJob(decoded) => assert_eq!(decoded, payload),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn set_paused_malformed_bool() {
        assert_eq!(
            parse_instruction(&[InstructionKind::SetPaused.as_u8(), 2]),
            Err(ProtocolError::InvalidBoolean)
        );
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic(data in prop::collection::vec(any::<u8>(), 0..400)) {
            let _ = parse_instruction(&data);
        }
    }
}
