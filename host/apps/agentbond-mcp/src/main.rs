use std::sync::Arc;

use agentbond_app::{AppConfig, ServiceCatalog};
use agentbond_mcp::AgentBondMcp;
use agentbond_sdk::{ChainReader, HttpChainReader, MockChainReader};
use rmcp::ServiceExt;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter("info")
        .init();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config/example.config.json".into());
    let cfg = AppConfig::load_file(&config_path)
        .map_err(|e| anyhow::anyhow!("invalid AgentBond config / RPC settings: {e}"))?;
    let catalog = ServiceCatalog::load_file(&cfg.catalog_path)?;

    let use_mock = std::env::var("AGENTBOND_USE_MOCK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let reader: Arc<dyn ChainReader> = if use_mock {
        info!("AGENTBOND_USE_MOCK set; using MockChainReader");
        Arc::new(MockChainReader::new())
    } else {
        Arc::new(
            HttpChainReader::new(&cfg.rpc_url, cfg.timeout())
                .map_err(|e| anyhow::anyhow!("invalid RPC configuration: {e}"))?,
        )
    };

    let server = AgentBondMcp {
        cfg,
        catalog,
        reader,
    };
    let transport = rmcp::transport::io::stdio();
    let service = server.serve(transport).await?;
    service.waiting().await?;
    Ok(())
}
