use std::sync::Arc;

use agentbond_app::{AppConfig, ServiceCatalog};
use agentbond_payments::{FacilitatorClient, PaymentCache};
use agentbond_sdk::ChainReader;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<AppConfig>,
    pub catalog: Arc<ServiceCatalog>,
    pub reader: Arc<dyn ChainReader>,
    pub facilitator: Arc<dyn FacilitatorClient>,
    pub payment_cache: Arc<PaymentCache>,
    pub requirements_issued_at: Arc<tokio::sync::Mutex<Option<i64>>>,
}
