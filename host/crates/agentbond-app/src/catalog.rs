use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::error::AppError;

const MAX_NAME: usize = 64;
const MAX_DESC: usize = 256;
const MAX_SERVICES: usize = 128;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceEntry {
    pub service_id: String,
    pub provider: String,
    pub name: String,
    pub description: String,
    pub request_schema: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x402_demo_route: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ServiceCatalog {
    services: Vec<ServiceEntry>,
}

impl ServiceCatalog {
    pub fn from_entries(entries: Vec<ServiceEntry>) -> Result<Self, AppError> {
        validate_entries(&entries)?;
        Ok(Self { services: entries })
    }

    pub fn load_file(path: impl AsRef<Path>) -> Result<Self, AppError> {
        let text = fs::read_to_string(path.as_ref())
            .map_err(|e| AppError::Config(format!("read catalog: {e}")))?;
        if text.len() > 256 * 1024 {
            return Err(AppError::Config("catalog file too large".into()));
        }
        let entries: Vec<ServiceEntry> = serde_json::from_str(&text)
            .map_err(|e| AppError::Config(format!("catalog json: {e}")))?;
        Self::from_entries(entries)
    }

    pub fn list(&self) -> &[ServiceEntry] {
        &self.services
    }

    pub fn get(&self, service_id: &str) -> Result<&ServiceEntry, AppError> {
        self.services
            .iter()
            .find(|s| s.service_id == service_id)
            .ok_or_else(|| AppError::NotFound(format!("service {service_id}")))
    }
}

fn validate_entries(entries: &[ServiceEntry]) -> Result<(), AppError> {
    if entries.len() > MAX_SERVICES {
        return Err(AppError::Validation("too many services".into()));
    }
    let mut ids = HashSet::new();
    for entry in entries {
        if entry.service_id.is_empty()
            || entry.provider.is_empty()
            || entry.name.is_empty()
            || entry.description.is_empty()
            || entry.request_schema.is_empty()
        {
            return Err(AppError::Validation(
                "service fields must be non-empty".into(),
            ));
        }
        if entry.name.len() > MAX_NAME || entry.description.len() > MAX_DESC {
            return Err(AppError::Validation("service text too long".into()));
        }
        if !ids.insert(entry.service_id.clone()) {
            return Err(AppError::Validation(format!(
                "duplicate service_id {}",
                entry.service_id
            )));
        }
        let _: Pubkey = entry
            .provider
            .parse()
            .map_err(|_| AppError::Validation(format!("invalid provider {}", entry.provider)))?;
    }
    Ok(())
}
