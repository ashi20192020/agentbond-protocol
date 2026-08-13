use agentbond_types::{AgentBondWorkReceiptV1, RECEIPT_ENCODED_LEN, encode_submit_receipt};
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

use crate::address::{config_pda, provider_pda};
use crate::error::SdkError;
use crate::plan::InstructionPlan;

pub fn validate_receipt(receipt: &AgentBondWorkReceiptV1) -> Result<(), SdkError> {
    let _ = receipt.encode()?;
    if receipt.expires_at < receipt.created_at {
        return Err(SdkError::InvalidInput(
            "receipt expires_at before created_at".into(),
        ));
    }
    Ok(())
}

pub fn receipt_digest(receipt: &AgentBondWorkReceiptV1) -> Result<[u8; 32], SdkError> {
    Ok(receipt.digest()?)
}

pub fn build_ed25519_verify_instruction(
    message: &[u8],
    public_key: &[u8; 32],
    signature: &[u8; 64],
) -> Result<Instruction, SdkError> {
    if message.len() != RECEIPT_ENCODED_LEN {
        return Err(SdkError::InvalidInput(format!(
            "receipt message must be {RECEIPT_ENCODED_LEN} bytes"
        )));
    }
    if public_key == &[0u8; 32] {
        return Err(SdkError::InvalidInput(
            "execution public key must be nonzero".into(),
        ));
    }
    const OFFSETS_START: usize = 2;
    const OFFSETS_SIZE: usize = 14;
    const DATA_START: usize = OFFSETS_START + OFFSETS_SIZE;
    let public_key_offset = DATA_START;
    let signature_offset = public_key_offset + 32;
    let message_data_offset = signature_offset + 64;
    let mut data = Vec::with_capacity(DATA_START + 32 + 64 + message.len());
    data.extend_from_slice(&[1u8, 0u8]);
    data.extend_from_slice(&(signature_offset as u16).to_le_bytes());
    data.extend_from_slice(&u16::MAX.to_le_bytes());
    data.extend_from_slice(&(public_key_offset as u16).to_le_bytes());
    data.extend_from_slice(&u16::MAX.to_le_bytes());
    data.extend_from_slice(&(message_data_offset as u16).to_le_bytes());
    data.extend_from_slice(&(message.len() as u16).to_le_bytes());
    data.extend_from_slice(&u16::MAX.to_le_bytes());
    data.extend_from_slice(public_key);
    data.extend_from_slice(signature);
    data.extend_from_slice(message);
    Ok(Instruction {
        program_id: Pubkey::from_str_const("Ed25519SigVerify111111111111111111111111111"),
        accounts: vec![],
        data,
    })
}

pub fn build_submit_receipt_plan(
    program_id: &Pubkey,
    job: &Pubkey,
    provider_authority: &Pubkey,
    receipt: &AgentBondWorkReceiptV1,
    execution_pubkey: &[u8; 32],
    signature: &[u8; 64],
) -> Result<InstructionPlan, SdkError> {
    build_submit_receipt_plan_at(
        program_id,
        job,
        provider_authority,
        receipt,
        execution_pubkey,
        signature,
        None,
    )
}

pub fn build_submit_receipt_plan_at(
    program_id: &Pubkey,
    job: &Pubkey,
    provider_authority: &Pubkey,
    receipt: &AgentBondWorkReceiptV1,
    execution_pubkey: &[u8; 32],
    signature: &[u8; 64],
    now: Option<i64>,
) -> Result<InstructionPlan, SdkError> {
    validate_receipt(receipt)?;
    if receipt.program_id != program_id.to_bytes() {
        return Err(SdkError::InvalidInput(
            "receipt program_id does not match requested program".into(),
        ));
    }
    if receipt.job != job.to_bytes() {
        return Err(SdkError::InvalidInput(
            "receipt job does not match supplied job".into(),
        ));
    }
    if receipt.provider != provider_authority.to_bytes() {
        return Err(SdkError::InvalidInput(
            "receipt provider does not match supplied provider".into(),
        ));
    }
    if execution_pubkey == &[0u8; 32] {
        return Err(SdkError::InvalidInput(
            "execution public key must be nonzero".into(),
        ));
    }
    if now.is_some_and(|ts| ts > receipt.expires_at) {
        return Err(SdkError::InvalidInput("receipt is expired".into()));
    }
    let encoded = receipt.encode()?;
    if encoded.len() != RECEIPT_ENCODED_LEN {
        return Err(SdkError::InvalidInput(format!(
            "canonical receipt must be {RECEIPT_ENCODED_LEN} bytes"
        )));
    }
    let ed = build_ed25519_verify_instruction(&encoded, execution_pubkey, signature)?;
    let config = config_pda(program_id)?.address;
    let provider = provider_pda(program_id, provider_authority)?.address;
    let instructions_sysvar = Pubkey::from_str_const("Sysvar1nstructions1111111111111111111111111");
    let submit = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new_readonly(config, false),
            AccountMeta::new_readonly(provider, false),
            AccountMeta::new(*job, false),
            AccountMeta::new_readonly(instructions_sysvar, false),
        ],
        data: encode_submit_receipt(receipt)?.to_vec(),
    };
    Ok(InstructionPlan::new(
        "submit_receipt",
        program_id,
        vec![ed, submit],
        vec![],
        Some(receipt.expires_at),
    ))
}
