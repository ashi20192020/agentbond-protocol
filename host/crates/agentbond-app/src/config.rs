use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use spl_token::ID as TOKEN_PROGRAM_ID;
use std::fs;
use std::path::Path;
use std::time::Duration;
use url::Url;

use crate::error::AppError;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppConfig {
    pub program_id: String,
    pub rpc_url: String,
    pub genesis_hash: String,
    pub settlement_mint: String,
    pub token_program: String,
    pub facilitator_url: String,
    pub merchant_pay_to: String,
    pub x402_fee_payer: String,
    pub x402_amount: String,
    pub x402_network: String,
    pub request_timeout_ms: u64,
    pub max_request_bytes: usize,
    pub bind_address: String,
    pub catalog_path: String,
}

impl AppConfig {
    pub fn load_file(path: impl AsRef<Path>) -> Result<Self, AppError> {
        let text = fs::read_to_string(path.as_ref())
            .map_err(|e| AppError::Config(format!("read config: {e}")))?;
        let mut cfg: Self = serde_json::from_str(&text)
            .map_err(|e| AppError::Config(format!("config json: {e}")))?;
        cfg.apply_env_overrides();
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("AGENTBOND_RPC_URL") {
            self.rpc_url = v;
        }
        if let Ok(v) = std::env::var("AGENTBOND_FACILITATOR_URL") {
            self.facilitator_url = v;
        }
        if let Ok(v) = std::env::var("AGENTBOND_BIND_ADDRESS") {
            self.bind_address = v;
        }
        if let Ok(v) = std::env::var("AGENTBOND_PROGRAM_ID") {
            self.program_id = v;
        }
    }

    pub fn validate(&self) -> Result<(), AppError> {
        parse_pk(&self.program_id, "program_id")?;
        parse_genesis_hash(&self.genesis_hash)?;
        parse_pk(&self.settlement_mint, "settlement_mint")?;
        let token = parse_pk(&self.token_program, "token_program")?;
        if token != TOKEN_PROGRAM_ID {
            return Err(AppError::Config(
                "token_program must be legacy SPL Token Program".into(),
            ));
        }
        parse_pk(&self.merchant_pay_to, "merchant_pay_to")?;
        parse_pk(&self.x402_fee_payer, "x402_fee_payer")?;
        reject_credentialed_url(&self.rpc_url, "rpc_url")?;
        reject_credentialed_url(&self.facilitator_url, "facilitator_url")?;
        if self
            .x402_amount
            .parse::<u64>()
            .ok()
            .filter(|v| *v > 0)
            .is_none()
        {
            return Err(AppError::Config(
                "x402_amount must be positive integer".into(),
            ));
        }
        if self.x402_network.is_empty()
            || self.x402_network.len() > 64
            || !self.x402_network.starts_with("solana:")
        {
            return Err(AppError::Config(
                "x402_network must be solana:<cluster> and bounded".into(),
            ));
        }
        if self.request_timeout_ms == 0 || self.request_timeout_ms > 120_000 {
            return Err(AppError::Config("request_timeout_ms out of range".into()));
        }
        if self.max_request_bytes == 0 || self.max_request_bytes > 1_048_576 {
            return Err(AppError::Config("max_request_bytes out of range".into()));
        }
        if self.bind_address.is_empty() || self.bind_address.len() > 128 {
            return Err(AppError::Config("bind_address invalid".into()));
        }
        if self.catalog_path.is_empty() || self.catalog_path.len() > 512 {
            return Err(AppError::Config("catalog_path required".into()));
        }
        Ok(())
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.request_timeout_ms)
    }

    pub fn program_pubkey(&self) -> Result<Pubkey, AppError> {
        parse_pk(&self.program_id, "program_id")
    }

    pub fn mint_pubkey(&self) -> Result<Pubkey, AppError> {
        parse_pk(&self.settlement_mint, "settlement_mint")
    }

    pub fn fee_payer_pubkey(&self) -> Result<Pubkey, AppError> {
        parse_pk(&self.x402_fee_payer, "x402_fee_payer")
    }
}

fn parse_pk(s: &str, label: &str) -> Result<Pubkey, AppError> {
    s.parse::<Pubkey>()
        .map_err(|_| AppError::Config(format!("invalid {label}")))
}

fn parse_genesis_hash(s: &str) -> Result<[u8; 32], AppError> {
    // Accept 32-byte hex or base58 Solana hash.
    let s = s.trim();
    if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        let mut out = [0u8; 32];
        for i in 0..32 {
            out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
                .map_err(|_| AppError::Config("invalid genesis_hash hex".into()))?;
        }
        return Ok(out);
    }
    let decoded = bs58::decode(s)
        .into_vec()
        .map_err(|_| AppError::Config("invalid genesis_hash".into()))?;
    if decoded.len() != 32 {
        return Err(AppError::Config(
            "genesis_hash must decode to 32 bytes".into(),
        ));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&decoded);
    Ok(out)
}

fn reject_credentialed_url(raw: &str, label: &str) -> Result<(), AppError> {
    let url = Url::parse(raw).map_err(|_| AppError::Config(format!("invalid {label}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::Config(format!("{label} must be http(s)")));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::Config(format!(
            "{label} must not contain credentials"
        )));
    }
    Ok(())
}
