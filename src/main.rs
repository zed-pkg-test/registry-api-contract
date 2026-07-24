mod auth;
mod config;
mod entities;
mod error;
mod files;
mod routes;
mod state;
mod storage;
mod tokens;
mod verify;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use migration::MigratorTrait;
use sea_orm::{ConnectOptions, Database};
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::state::AppState;
use crate::storage::ArtifactStore;
use crate::verify::TagVerifier;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("create-token") {
        return tokens::create_token(&args[2..]).await;
    }

    let cfg = Config::from_env()?;
    let mut connect_opts = ConnectOptions::new(cfg.database_url.clone());
    connect_opts
        .max_connections(cfg.db_max_connections)
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(8))
        .sqlx_logging(false);
    let db = Database::connect(connect_opts)
        .await
        .context("failed to connect to DATABASE_URL")?;
    if cfg.auto_migrate {
        migration::Migrator::up(&db, None)
            .await
            .context("migrations failed")?;
    }
    let store = ArtifactStore::from_config(&cfg.storage).await?;
    let state = Arc::new(AppState {
        db,
        store,
        verifier: TagVerifier::new(cfg.verify_tags),
        public_base_url: cfg.public_base_url.trim_end_matches('/').to_string(),
        max_orgs_per_token: cfg.max_orgs_per_token,
    });

    let app = routes::router(state, cfg.max_artifact_bytes);
    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr).await?;
    tracing::info!("zed-api-server listening on {}", cfg.bind_addr);
    axum::serve(listener, app).await?;
    Ok(())
}
