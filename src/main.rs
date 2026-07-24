mod auth;
mod config;
mod entities;
mod error;
mod files;
mod routes;
mod state;
mod storage;
mod verify;

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use migration::MigratorTrait;
use sea_orm::{ActiveValue, Database, EntityTrait};
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
        return create_token(&args[2..]).await;
    }

    let cfg = Config::from_env()?;
    let db = Database::connect(&cfg.database_url)
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
    });

    let app = routes::router(state, cfg.max_artifact_bytes);
    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr).await?;
    tracing::info!("zed-api-server listening on {}", cfg.bind_addr);
    axum::serve(listener, app).await?;
    Ok(())
}

/// `zed-api-server create-token --name <name> [--org <slug>]`
/// Prints the plaintext token exactly once; only its sha256 is stored.
async fn create_token(args: &[String]) -> Result<()> {
    let mut name = None;
    let mut org_slug = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--name" => name = iter.next().cloned(),
            "--org" => org_slug = iter.next().cloned(),
            other => bail!("unknown argument `{other}`"),
        }
    }
    let name = name.context("--name is required")?;

    let cfg = Config::from_env()?;
    let db = Database::connect(&cfg.database_url).await?;
    migration::Migrator::up(&db, None).await?;

    let org_id = match org_slug {
        Some(slug) => {
            use sea_orm::{ColumnTrait, QueryFilter};
            let org_row = entities::org::Entity::find()
                .filter(entities::org::Column::Slug.eq(&slug))
                .one(&db)
                .await?;
            let org_row = match org_row {
                Some(existing) => existing,
                None => {
                    entities::org::ActiveModel {
                        id: ActiveValue::Set(uuid::Uuid::new_v4()),
                        slug: ActiveValue::Set(slug.clone()),
                        created_at: ActiveValue::Set(chrono::Utc::now()),
                    }
                    .insert(&db)
                    .await?
                }
            };
            Some(org_row.id)
        }
        None => None,
    };

    let plaintext = format!(
        "zpkg_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    use sea_orm::ActiveModelTrait;
    entities::token::ActiveModel {
        id: ActiveValue::Set(uuid::Uuid::new_v4()),
        name: ActiveValue::Set(name.clone()),
        token_hash: ActiveValue::Set(auth::hash_token(&plaintext)),
        org_id: ActiveValue::Set(org_id),
        created_at: ActiveValue::Set(chrono::Utc::now()),
    }
    .insert(&db)
    .await?;

    println!("token `{name}` created; save it now, it is shown exactly once:");
    println!("{plaintext}");
    Ok(())
}
