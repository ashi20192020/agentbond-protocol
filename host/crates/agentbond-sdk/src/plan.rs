use serde::{Deserialize, Serialize};
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

use crate::error::SdkError;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountMetaPlan {
    pub pubkey: String,
    pub is_signer: bool,
    pub is_writable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannedInstruction {
    pub program_id: String,
    pub accounts: Vec<AccountMetaPlan>,
    pub data_base64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstructionPlan {
    pub action: String,
    pub program_id: String,
    pub instructions: Vec<PlannedInstruction>,
    pub required_signers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

impl InstructionPlan {
    pub fn new(
        action: impl Into<String>,
        program_id: &Pubkey,
        instructions: Vec<Instruction>,
        required_signers: Vec<Pubkey>,
        expires_at: Option<i64>,
    ) -> Self {
        Self {
            action: action.into(),
            program_id: program_id.to_string(),
            instructions: instructions
                .into_iter()
                .map(PlannedInstruction::from)
                .collect(),
            required_signers: required_signers
                .into_iter()
                .map(|p| p.to_string())
                .collect(),
            expires_at,
        }
    }

    pub fn to_json(&self) -> Result<String, SdkError> {
        serde_json::to_string_pretty(self).map_err(Into::into)
    }

    pub fn from_json(s: &str) -> Result<Self, SdkError> {
        serde_json::from_str(s).map_err(Into::into)
    }

    pub fn to_solana_instructions(&self) -> Result<Vec<Instruction>, SdkError> {
        self.instructions
            .iter()
            .map(PlannedInstruction::to_instruction)
            .collect()
    }
}

impl From<Instruction> for PlannedInstruction {
    fn from(ix: Instruction) -> Self {
        Self {
            program_id: ix.program_id.to_string(),
            accounts: ix
                .accounts
                .into_iter()
                .map(|a| AccountMetaPlan {
                    pubkey: a.pubkey.to_string(),
                    is_signer: a.is_signer,
                    is_writable: a.is_writable,
                })
                .collect(),
            data_base64: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                ix.data,
            ),
        }
    }
}

impl PlannedInstruction {
    pub fn to_instruction(&self) -> Result<Instruction, SdkError> {
        let program_id = self
            .program_id
            .parse::<Pubkey>()
            .map_err(|_| SdkError::InvalidPubkey(self.program_id.clone()))?;
        let accounts = self
            .accounts
            .iter()
            .map(|a| {
                let pubkey = a
                    .pubkey
                    .parse::<Pubkey>()
                    .map_err(|_| SdkError::InvalidPubkey(a.pubkey.clone()))?;
                Ok(if a.is_writable {
                    if a.is_signer {
                        AccountMeta::new(pubkey, true)
                    } else {
                        AccountMeta::new(pubkey, false)
                    }
                } else if a.is_signer {
                    AccountMeta::new_readonly(pubkey, true)
                } else {
                    AccountMeta::new_readonly(pubkey, false)
                })
            })
            .collect::<Result<Vec<_>, SdkError>>()?;
        let data = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &self.data_base64,
        )
        .map_err(|e| SdkError::InvalidInput(format!("invalid instruction data base64: {e}")))?;
        Ok(Instruction {
            program_id,
            accounts,
            data,
        })
    }
}
