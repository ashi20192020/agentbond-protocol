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

    let data = ed25519_ix.get_instruction_data();
    if data.len() < ED25519_SIGNATURE_OFFSETS_START {
        return Err(fail(ProtocolError::InvalidEd25519Instruction));
    }

    let num_signatures = data[0] as usize;
    if num_signatures != 1 {
        return Err(fail(ProtocolError::InvalidEd25519Instruction));
    }

    let offsets_end = ED25519_SIGNATURE_OFFSETS_START
        .checked_add(ED25519_SIGNATURE_OFFSETS_SERIALIZED_SIZE)
        .ok_or_else(|| fail(ProtocolError::InvalidEd25519Instruction))?;
    if data.len() < offsets_end {
        return Err(fail(ProtocolError::InvalidEd25519Instruction));
    }

    let offsets = &data[ED25519_SIGNATURE_OFFSETS_START..offsets_end];
    let signature_offset = read_u16(offsets, 0)?;
    let signature_instruction_index = read_u16(offsets, 2)?;
    let public_key_offset = read_u16(offsets, 4)?;
    let public_key_instruction_index = read_u16(offsets, 6)?;
    let message_data_offset = read_u16(offsets, 8)?;
    let message_data_size = read_u16(offsets, 10)?;
    let message_instruction_index = read_u16(offsets, 12)?;

    let ed25519_index = instructions
        .load_current_index()
        .checked_sub(1)
        .ok_or_else(|| fail(ProtocolError::InvalidEd25519Instruction))?;

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

    // Signature presence is enforced by the precompile; still bounds-check offsets.
    let _signature = slice_checked(
        data,
        signature_offset as usize,
        ED25519_SIGNATURE_SERIALIZED_SIZE,
    )?;

    if message_data_size as usize != RECEIPT_ENCODED_LEN {
        return Err(fail(ProtocolError::InvalidEd25519Instruction));
    }
    let signed_message = slice_checked(
        data,
        message_data_offset as usize,
        message_data_size as usize,
    )?;
    if signed_message != message {
        return Err(fail(ProtocolError::InvalidSignature));
    }

    Ok(pubkey)
}

fn require_self_index(
    index: u16,
    ed25519_index: u16,
) -> Result<(), pinocchio::error::ProgramError> {
    if index == ED25519_CURRENT_INSTRUCTION_INDEX || index == ed25519_index {
        Ok(())
    } else {
        Err(fail(ProtocolError::InvalidEd25519Instruction))
    }
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, pinocchio::error::ProgramError> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| fail(ProtocolError::InvalidEd25519Instruction))?;
    let bytes = data
        .get(offset..end)
        .ok_or_else(|| fail(ProtocolError::InvalidEd25519Instruction))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn slice_checked(
    data: &[u8],
    offset: usize,
    len: usize,
) -> Result<&[u8], pinocchio::error::ProgramError> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| fail(ProtocolError::InvalidEd25519Instruction))?;
    data.get(offset..end)
        .ok_or_else(|| fail(ProtocolError::InvalidEd25519Instruction))
}
