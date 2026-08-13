use std::sync::Arc;

use agentbond_app::{AppConfig, ServiceCatalog};
use agentbond_payments::{ChallengeStore, FacilitatorClient, SettlementStore};
use agentbond_sdk::ChainReader;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<AppConfig>,
    pub catalog: Arc<ServiceCatalog>,
    pub reader: Arc<dyn ChainReader>,
    pub facilitator: Arc<dyn FacilitatorClient>,
    pub challenges: Arc<ChallengeStore>,
    pub settlements: Arc<SettlementStore>,
}
