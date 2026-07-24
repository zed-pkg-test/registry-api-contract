//! `zed-api-server create-token --name <name> [--org <slug>]`
//! Mints a registry token: prints the plaintext exactly once, stores only
//! its sha256 (see `auth`). With `--org`, the token is scoped to that org
//! (created on the fly if needed); without, it is an admin token.

use anyhow::{Context, Result, bail};
use migration::MigratorTrait;
use sea_orm::{ActiveModelTrait, ActiveValue, ColumnTrait, Database, EntityTrait, QueryFilter};

use crate::auth::hash_token;
use crate::config::Config;
use crate::entities::{org, token};

pub async fn create_token(args: &[String]) -> Result<()> {
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
        Some(slug) => Some(find_or_create_org(&db, &slug).await?.id),
        None => None,
    };

    let plaintext = format!(
        "zpkg_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    token::ActiveModel {
        id: ActiveValue::Set(uuid::Uuid::new_v4()),
        name: ActiveValue::Set(name.clone()),
        token_hash: ActiveValue::Set(hash_token(&plaintext)),
        org_id: ActiveValue::Set(org_id),
        created_at: ActiveValue::Set(chrono::Utc::now()),
    }
    .insert(&db)
    .await?;

    println!("token `{name}` created; save it now, it is shown exactly once:");
    println!("{plaintext}");
    Ok(())
}

async fn find_or_create_org(db: &sea_orm::DatabaseConnection, slug: &str) -> Result<org::Model> {
    if let Some(existing) = org::Entity::find()
        .filter(org::Column::Slug.eq(slug))
        .one(db)
        .await?
    {
        return Ok(existing);
    }
    Ok(org::ActiveModel {
        id: ActiveValue::Set(uuid::Uuid::new_v4()),
        slug: ActiveValue::Set(slug.to_string()),
        created_at: ActiveValue::Set(chrono::Utc::now()),
    }
    .insert(db)
    .await?)
}
