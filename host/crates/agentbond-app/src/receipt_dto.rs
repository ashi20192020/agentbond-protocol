use agentbond_types::AgentBondWorkReceiptV1;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReceiptDto {
    pub program_id_hex: String,
    pub genesis_hash_hex: String,
    pub job_hex: String,
    pub buyer_hex: String,
    pub provider_hex: String,
    pub request_hash_hex: String,
    pub result_hash_hex: String,
    pub artifact_hash_hex: String,
    pub software_hash_hex: String,
    pub job_nonce: u64,
    pub created_at: i64,
    pub expires_at: i64,
}

impl ReceiptDto {
    pub fn to_receipt(&self) -> Result<AgentBondWorkReceiptV1, AppError> {
        Ok(AgentBondWorkReceiptV1 {
            program_id: hex32(&self.program_id_hex, "program_id")?,
            genesis_hash: hex32(&self.genesis_hash_hex, "genesis_hash")?,
            job: hex32(&self.job_hex, "job")?,
            buyer: hex32(&self.buyer_hex, "buyer")?,
            provider: hex32(&self.provider_hex, "provider")?,
            request_hash: hex32(&self.request_hash_hex, "request_hash")?,
            result_hash: hex32(&self.result_hash_hex, "result_hash")?,
            artifact_hash: hex32(&self.artifact_hash_hex, "artifact_hash")?,
            software_hash: hex32(&self.software_hash_hex, "software_hash")?,
            job_nonce: self.job_nonce,
            created_at: self.created_at,
            expires_at: self.expires_at,
        })
    }

    pub fn from_receipt(r: &AgentBondWorkReceiptV1) -> Self {
        Self {
            program_id_hex: hex_encode(&r.program_id),
            genesis_hash_hex: hex_encode(&r.genesis_hash),
            job_hex: hex_encode(&r.job),
            buyer_hex: hex_encode(&r.buyer),
            provider_hex: hex_encode(&r.provider),
            request_hash_hex: hex_encode(&r.request_hash),
            result_hash_hex: hex_encode(&r.result_hash),
            artifact_hash_hex: hex_encode(&r.artifact_hash),
            software_hash_hex: hex_encode(&r.software_hash),
            job_nonce: r.job_nonce,
            created_at: r.created_at,
            expires_at: r.expires_at,
        }
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex32(s: &str, label: &str) -> Result<[u8; 32], AppError> {
    let s = s.trim().strip_prefix("0x").unwrap_or(s.trim());
    if s.len() != 64 {
        return Err(AppError::Validation(format!(
            "{label} must be 32 bytes hex"
        )));
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
            .map_err(|_| AppError::Validation("invalid hex".into()))?;
    }
    Ok(out)
}
