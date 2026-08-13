use std::net::SocketAddr;
use std::sync::Arc;

use agentbond_app::{AppConfig, ServiceCatalog};
use agentbond_gateway::{AppState, router};
use agentbond_payments::{
    FacilitatorClient, HttpFacilitatorClient, MockFacilitatorClient, PaymentCache,
};
use agentbond_sdk::{ChainReader, HttpChainReader, MockChainReader};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .json()
        .init();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config/example.config.json".into());
    let cfg = AppConfig::load_file(&config_path)?;
    let catalog = ServiceCatalog::load_file(&cfg.catalog_path)?;
    let timeout = cfg.timeout();

    let use_mock = std::env::var("AGENTBOND_USE_MOCK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let (reader, facilitator): (Arc<dyn ChainReader>, Arc<dyn FacilitatorClient>) = if use_mock {
        (
            Arc::new(MockChainReader::new()),
            Arc::new(MockFacilitatorClient::new()),
        )
    } else {
        (
            Arc::new(HttpChainReader::new(&cfg.rpc_url, timeout)?),
            Arc::new(HttpFacilitatorClient::new(&cfg.facilitator_url, timeout)?),
        )
    };

    let state = AppState {
        cfg: Arc::new(cfg.clone()),
        catalog: Arc::new(catalog),
        reader,
        facilitator,
        payment_cache: Arc::new(PaymentCache::new()),
        requirements_issued_at: Arc::new(tokio::sync::Mutex::new(None)),
    };

    let app = router(state, cfg.max_request_bytes, timeout);
    let addr: SocketAddr = cfg.bind_address.parse()?;
    info!(%addr, "agentbond-gateway listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    warn!("shutdown signal received");
}
