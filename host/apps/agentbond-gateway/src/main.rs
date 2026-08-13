use std::net::SocketAddr;
use std::sync::Arc;

use agentbond_app::{AppConfig, ServiceCatalog};
use agentbond_db::{Db, PgChallengeStore, PgSettlementStore, redact_db_url};
use agentbond_gateway::metrics::{MeteredSettlementStore, PaymentMetrics};
use agentbond_gateway::{AppState, router};
use agentbond_payments::{
    FacilitatorClient, HttpFacilitatorClient, MemoryChallengeStore, MemorySettlementStore,
    MockFacilitatorClient,
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
            Arc::new(HttpFacilitatorClient::new(
                &cfg.facilitator_url,
                timeout,
                &cfg.x402_network,
                Some(cfg.x402_fee_payer.clone()),
            )?),
        )
    };

    let payment_metrics = Arc::new(PaymentMetrics::new()?);

    let (challenges, settlements, db) = if use_mock {
        (
            Arc::new(MemoryChallengeStore::new()) as Arc<dyn agentbond_payments::ChallengeStore>,
            Arc::new(MeteredSettlementStore::new(
                Arc::new(MemorySettlementStore::new()),
                payment_metrics.clone(),
            )) as Arc<dyn agentbond_payments::SettlementStore>,
            None,
        )
    } else {
        let database_url = std::env::var("AGENTBOND_DATABASE_URL").map_err(|_| {
            anyhow::anyhow!("AGENTBOND_DATABASE_URL is required unless AGENTBOND_USE_MOCK=1")
        })?;
        info!(db = %redact_db_url(&database_url), "database configured");
        let db = Arc::new(Db::connect(&database_url).await?);
        db.migrations_status()
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        (
            Arc::new(PgChallengeStore::new(db.pool().clone()))
                as Arc<dyn agentbond_payments::ChallengeStore>,
            Arc::new(MeteredSettlementStore::new(
                Arc::new(PgSettlementStore::new(db.pool().clone())),
                payment_metrics.clone(),
            )) as Arc<dyn agentbond_payments::SettlementStore>,
            Some(db),
        )
    };

    let state = AppState {
        cfg: Arc::new(cfg.clone()),
        catalog: Arc::new(catalog),
        reader,
        facilitator,
        challenges,
        settlements,
        db,
        payment_metrics,
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
