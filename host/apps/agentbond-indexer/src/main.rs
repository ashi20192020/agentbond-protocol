use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use agentbond_db::{Db, ProjectionRepo, redact_db_url};
use agentbond_indexer::{
    IndexerEngine, IndexerMetrics, RpcGapBackfill, YellowstoneConfig, YellowstoneSource,
    replay_fixture,
};
use axum::Router;
use axum::routing::get;
use clap::{Parser, Subcommand};
use solana_pubkey::Pubkey;
use tracing::{info, warn};

#[derive(Parser)]
#[command(name = "agentbond-indexer")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Migrate,
    Run,
    Replay {
        #[arg(long)]
        fixture: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .json()
        .init();

    let cli = Cli::parse();
    let database_url = std::env::var("AGENTBOND_DATABASE_URL")
        .map_err(|_| anyhow::anyhow!("AGENTBOND_DATABASE_URL is required"))?;
    info!(db = %redact_db_url(&database_url), "database configured");
    let db = Arc::new(Db::connect(&database_url).await?);

    match cli.command {
        Commands::Migrate => {
            db.migrate().await?;
            info!("migrations applied");
        }
        Commands::Replay { fixture } => {
            db.migrate().await?;
            let metrics = IndexerMetrics::new().map_err(|e| anyhow::anyhow!(e))?;
            replay_fixture(db, fixture, &metrics).await?;
            info!("fixture replay complete");
        }
        Commands::Run => {
            db.migrate().await?;
            let metrics = IndexerMetrics::new().map_err(|e| anyhow::anyhow!(e))?;
            let metrics_bg = metrics.clone();
            let db_health = db.clone();
            tokio::spawn(async move {
                if let Err(e) = serve_ops(metrics_bg, db_health).await {
                    warn!(error = %e, "ops server stopped");
                }
            });
            let ys = YellowstoneConfig::from_env()?;
            let rpc_url = std::env::var("AGENTBOND_RPC_URL")
                .map_err(|_| anyhow::anyhow!("AGENTBOND_RPC_URL is required for run"))?;
            let program_id: Pubkey = std::env::var("AGENTBOND_PROGRAM_ID")
                .map_err(|_| anyhow::anyhow!("AGENTBOND_PROGRAM_ID is required"))?
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid AGENTBOND_PROGRAM_ID: {e}"))?;
            let backfill = Arc::new(RpcGapBackfill::new(
                &rpc_url,
                program_id,
                Duration::from_secs(10),
            )?);
            let source = YellowstoneSource::new(ys, metrics.clone());
            let engine = IndexerEngine::new(db, metrics).with_backfill(backfill);
            engine.run_source(&source).await?;
        }
    }
    Ok(())
}

async fn serve_ops(metrics: IndexerMetrics, db: Arc<Db>) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/health/live", get(|| async { "ok" }))
        .route(
            "/health/ready",
            get({
                let db = db.clone();
                move || {
                    let db = db.clone();
                    async move {
                        let repo = ProjectionRepo::new(db.pool().clone());
                        match (
                            db.health().await,
                            db.migrations_current().await,
                            repo.checkpoint().await,
                        ) {
                            (Ok(()), Ok(true), Ok(_)) => (axum::http::StatusCode::OK, "ready"),
                            _ => (axum::http::StatusCode::SERVICE_UNAVAILABLE, "not ready"),
                        }
                    }
                }
            }),
        )
        .route(
            "/metrics",
            get({
                let metrics = metrics.clone();
                move || {
                    let metrics = metrics.clone();
                    async move { metrics.render() }
                }
            }),
        );
    let addr: SocketAddr = std::env::var("AGENTBOND_INDEXER_OPS_BIND")
        .unwrap_or_else(|_| "127.0.0.1:9100".into())
        .parse()?;
    info!(%addr, "indexer ops listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
