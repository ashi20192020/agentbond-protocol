use agentbond_types::{parse_instruction, InstructionKind, ProtocolError};
use pinocchio::account::AccountView;
use pinocchio::address::Address;
use pinocchio::error::{ProgramError, ProgramResult};

use crate::error::from_protocol;

pub fn process(
    _program_id: &Address,
    _accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let instruction = parse_instruction(instruction_data).map_err(from_protocol)?;
    dispatch(instruction.kind)
}

fn dispatch(kind: InstructionKind) -> ProgramResult {
    match kind {
        InstructionKind::InitializeConfig
        | InstructionKind::RegisterProvider
        | InstructionKind::AddExecutionKey
        | InstructionKind::RevokeExecutionKey
        | InstructionKind::DepositBond
        | InstructionKind::WithdrawBond
        | InstructionKind::CreateJob
        | InstructionKind::FundJob
        | InstructionKind::AcceptJob
        | InstructionKind::SubmitReceipt
        | InstructionKind::AcceptWork
        | InstructionKind::ChallengeWork
        | InstructionKind::ResolveTimeoutSettle
        | InstructionKind::ResolveTimeoutRefund
        | InstructionKind::ExpireUnfunded
        | InstructionKind::ExpireUnaccepted
        | InstructionKind::SlashBond
        | InstructionKind::CloseJob => Err(from_protocol(ProtocolError::InstructionNotImplemented)),
    }
}

pub fn process_raw(instruction_data: &[u8]) -> Result<(), ProgramError> {
    let instruction = parse_instruction(instruction_data).map_err(from_protocol)?;
    dispatch(instruction.kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentbond_types::InstructionKind;
    use proptest::prelude::*;

    #[test]
    fn recognized_instructions_return_not_implemented() {
        for value in 0u8..=17 {
            let kind = InstructionKind::from_u8(value).expect("kind");
            let err = process_raw(&[kind.as_u8()]).expect_err("must not succeed");
            assert_eq!(
                err,
                ProgramError::Custom(ProtocolError::InstructionNotImplemented.code())
            );
        }
    }

    #[test]
    fn empty_data_rejected() {
        assert_eq!(
            process_raw(&[]),
            Err(ProgramError::Custom(
                ProtocolError::EmptyInstructionData.code()
            ))
        );
    }

    #[test]
    fn unknown_discriminator_rejected() {
        assert_eq!(
            process_raw(&[200]),
            Err(ProgramError::Custom(
                ProtocolError::UnknownInstruction.code()
            ))
        );
    }

    #[test]
    fn incorrect_payload_length_rejected() {
        assert_eq!(
            process_raw(&[InstructionKind::FundJob.as_u8(), 1, 2, 3]),
            Err(ProgramError::Custom(
                ProtocolError::InvalidInstructionLength.code()
            ))
        );
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic(data in prop::collection::vec(any::<u8>(), 0..64)) {
            let _ = process_raw(&data);
        }
    }
}
