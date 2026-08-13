use crate::error::ProtocolError;

pub const INSTRUCTION_DISCRIMINATOR_LEN: usize = 1;

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
            _ => Err(ProtocolError::UnknownInstruction),
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn expected_payload_len(self) -> usize {
        0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Instruction {
    pub kind: InstructionKind,
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

    Ok(Instruction { kind })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn known_discriminator_parsing() {
        for value in 0u8..=17 {
            let kind = InstructionKind::from_u8(value).expect("known kind");
            let parsed = parse_instruction(&[kind.as_u8()]).expect("parse");
            assert_eq!(parsed.kind, kind);
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
            parse_instruction(&[18]),
            Err(ProtocolError::UnknownInstruction)
        );
        assert_eq!(
            parse_instruction(&[255]),
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

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic(data in prop::collection::vec(any::<u8>(), 0..64)) {
            let _ = parse_instruction(&data);
        }
    }
}
