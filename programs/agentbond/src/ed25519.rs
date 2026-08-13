use agentbond_types::{ProtocolError, RECEIPT_ENCODED_LEN};
use pinocchio::account::AccountView;
use pinocchio::sysvars::instructions::Instructions;

use crate::constants::{
    ED25519_CURRENT_INSTRUCTION_INDEX, ED25519_PROGRAM_ID, ED25519_PUBKEY_SERIALIZED_SIZE,
    ED25519_SIGNATURE_OFFSETS_SERIALIZED_SIZE, ED25519_SIGNATURE_OFFSETS_START,
    ED25519_SIGNATURE_SERIALIZED_SIZE,
};
use crate::error::fail;

/// Validate that the immediately preceding instruction is a well-formed Ed25519
/// precompile verifying `message`. Returns the verified public key.
pub fn verify_preceding_ed25519(
    instructions_sysvar: &AccountView,
    message: &[u8],
) -> Result<[u8; 32], pinocchio::error::ProgramError> {
    if message.len() != RECEIPT_ENCODED_LEN {
        return Err(fail(ProtocolError::InvalidReceiptLength));
    }

    let instructions = Instructions::try_from(instructions_sysvar)
        .map_err(|_| fail(ProtocolError::InvalidEd25519Instruction))?;

    let ed25519_ix = instructions
        .get_instruction_relative(-1)
        .map_err(|_| fail(ProtocolError::MissingEd25519Instruction))?;

    if ed25519_ix.get_program_id() != &ED25519_PROGRAM_ID {
        return Err(fail(ProtocolError::MissingEd25519Instruction));
    }

    let ed25519_index = instructions
        .load_current_index()
        .checked_sub(1)
        .ok_or_else(|| fail(ProtocolError::InvalidEd25519Instruction))?;

    parse_ed25519_instruction_data(ed25519_ix.get_instruction_data(), message, ed25519_index)
        .map_err(fail)
}

/// Pure layout parser for Ed25519 precompile instruction data.
/// Safe on arbitrary bytes: returns `Err`, never panics.
pub fn parse_ed25519_instruction_data(
    data: &[u8],
    message: &[u8],
    ed25519_index: u16,
) -> Result<[u8; 32], ProtocolError> {
    if message.len() != RECEIPT_ENCODED_LEN {
        return Err(ProtocolError::InvalidReceiptLength);
    }
    if data.len() < ED25519_SIGNATURE_OFFSETS_START {
        return Err(ProtocolError::InvalidEd25519Instruction);
    }

    let num_signatures = data[0] as usize;
    if num_signatures != 1 {
        return Err(ProtocolError::InvalidEd25519Instruction);
    }

    let offsets_end = ED25519_SIGNATURE_OFFSETS_START
        .checked_add(ED25519_SIGNATURE_OFFSETS_SERIALIZED_SIZE)
        .ok_or(ProtocolError::InvalidEd25519Instruction)?;
    if data.len() < offsets_end {
        return Err(ProtocolError::InvalidEd25519Instruction);
    }

    let offsets = &data[ED25519_SIGNATURE_OFFSETS_START..offsets_end];
    let signature_offset = read_u16(offsets, 0)?;
    let signature_instruction_index = read_u16(offsets, 2)?;
    let public_key_offset = read_u16(offsets, 4)?;
    let public_key_instruction_index = read_u16(offsets, 6)?;
    let message_data_offset = read_u16(offsets, 8)?;
    let message_data_size = read_u16(offsets, 10)?;
    let message_instruction_index = read_u16(offsets, 12)?;

    require_self_index(signature_instruction_index, ed25519_index)?;
    require_self_index(public_key_instruction_index, ed25519_index)?;
    require_self_index(message_instruction_index, ed25519_index)?;

    let pubkey_bytes = slice_checked(
        data,
        public_key_offset as usize,
        ED25519_PUBKEY_SERIALIZED_SIZE,
    )?;
    let mut pubkey = [0u8; 32];
    pubkey.copy_from_slice(pubkey_bytes);

    let _signature = slice_checked(
        data,
        signature_offset as usize,
        ED25519_SIGNATURE_SERIALIZED_SIZE,
    )?;

    if message_data_size as usize != RECEIPT_ENCODED_LEN {
        return Err(ProtocolError::InvalidEd25519Instruction);
    }
    let signed_message = slice_checked(
        data,
        message_data_offset as usize,
        message_data_size as usize,
    )?;
    if signed_message != message {
        return Err(ProtocolError::InvalidSignature);
    }

    Ok(pubkey)
}

fn require_self_index(index: u16, ed25519_index: u16) -> Result<(), ProtocolError> {
    if index == ED25519_CURRENT_INSTRUCTION_INDEX || index == ed25519_index {
        Ok(())
    } else {
        Err(ProtocolError::InvalidEd25519Instruction)
    }
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, ProtocolError> {
    let end = offset
        .checked_add(2)
        .ok_or(ProtocolError::InvalidEd25519Instruction)?;
    let bytes = data
        .get(offset..end)
        .ok_or(ProtocolError::InvalidEd25519Instruction)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn slice_checked(data: &[u8], offset: usize, len: usize) -> Result<&[u8], ProtocolError> {
    let end = offset
        .checked_add(len)
        .ok_or(ProtocolError::InvalidEd25519Instruction)?;
    data.get(offset..end)
        .ok_or(ProtocolError::InvalidEd25519Instruction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentbond_types::RECEIPT_ENCODED_LEN;
    use proptest::prelude::*;

    #[test]
    fn truncated_header_rejected() {
        let message = [0u8; RECEIPT_ENCODED_LEN];
        assert_eq!(
            parse_ed25519_instruction_data(&[], &message, 0),
            Err(ProtocolError::InvalidEd25519Instruction)
        );
        assert_eq!(
            parse_ed25519_instruction_data(&[1], &message, 0),
            Err(ProtocolError::InvalidEd25519Instruction)
        );
    }

    #[test]
    fn zero_and_multi_sig_rejected() {
        let message = [0u8; RECEIPT_ENCODED_LEN];
        let mut data = vec![0u8; 16];
        assert_eq!(
            parse_ed25519_instruction_data(&data, &message, 0),
            Err(ProtocolError::InvalidEd25519Instruction)
        );
        data[0] = 2;
        assert_eq!(
            parse_ed25519_instruction_data(&data, &message, 0),
            Err(ProtocolError::InvalidEd25519Instruction)
        );
    }

    #[test]
    fn out_of_bounds_offsets_rejected() {
        let message = [0u8; RECEIPT_ENCODED_LEN];
        let mut data = vec![0u8; 16];
        data[0] = 1;
        // signature_offset = 0xffff
        data[2] = 0xff;
        data[3] = 0xff;
        assert_eq!(
            parse_ed25519_instruction_data(&data, &message, 0),
            Err(ProtocolError::InvalidEd25519Instruction)
        );
    }

    proptest! {
        #[test]
        fn arbitrary_ed25519_bytes_never_panic(data in prop::collection::vec(any::<u8>(), 0..512)) {
            let message = [7u8; RECEIPT_ENCODED_LEN];
            let _ = parse_ed25519_instruction_data(&data, &message, 0);
            let _ = parse_ed25519_instruction_data(&data, &message, u16::MAX);
        }
    }
}
