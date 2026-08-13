use std::sync::Arc;

use agentbond_app::{AppConfig, ServiceCatalog};
use agentbond_payments::{ChallengeStore, FacilitatorClient, SettlementStore};
use agentbond_sdk::ChainReader;

use crate::metrics::PaymentMetrics;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<AppConfig>,
    pub catalog: Arc<ServiceCatalog>,
    pub reader: Arc<dyn ChainReader>,
    pub facilitator: Arc<dyn FacilitatorClient>,
    pub challenges: Arc<dyn ChallengeStore>,
    pub settlements: Arc<dyn SettlementStore>,
    pub db: Option<Arc<agentbond_db::Db>>,
    pub payment_metrics: Arc<PaymentMetrics>,
}
