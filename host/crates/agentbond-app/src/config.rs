use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use std::fs;
use std::path::Path;
use std::time::Duration;

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
        parse_pk(&self.settlement_mint, "settlement_mint")?;
        parse_pk(&self.token_program, "token_program")?;
        parse_pk(&self.merchant_pay_to, "merchant_pay_to")?;
        if self.genesis_hash.len() != 64
            || !self.genesis_hash.chars().all(|c| c.is_ascii_hexdigit())
        {
            // allow base58 or hex — require non-empty bounded
            if self.genesis_hash.is_empty() || self.genesis_hash.len() > 88 {
                return Err(AppError::Config("invalid genesis_hash".into()));
            }
        }
        if !(self.rpc_url.starts_with("http://") || self.rpc_url.starts_with("https://")) {
            return Err(AppError::Config("rpc_url must be http(s)".into()));
        }
        if !(self.facilitator_url.starts_with("http://")
            || self.facilitator_url.starts_with("https://"))
        {
            return Err(AppError::Config("facilitator_url must be http(s)".into()));
        }
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
        if self.x402_network.is_empty() || !self.x402_network.starts_with("solana:") {
            return Err(AppError::Config(
                "x402_network must be solana:<cluster>".into(),
            ));
        }
        if self.request_timeout_ms == 0 || self.request_timeout_ms > 120_000 {
            return Err(AppError::Config("request_timeout_ms out of range".into()));
        }
        if self.max_request_bytes == 0 || self.max_request_bytes > 1_048_576 {
            return Err(AppError::Config("max_request_bytes out of range".into()));
        }
        if self.bind_address.is_empty() {
            return Err(AppError::Config("bind_address required".into()));
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
}

fn parse_pk(s: &str, label: &str) -> Result<Pubkey, AppError> {
    s.parse::<Pubkey>()
        .map_err(|_| AppError::Config(format!("invalid {label}")))
}
