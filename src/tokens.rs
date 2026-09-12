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
    let mut role = "owner".to_string();
    let mut expires_in_days: Option<i64> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--name" => name = iter.next().cloned(),
            "--org" => org_slug = iter.next().cloned(),
            "--role" => {
                role = iter.next().cloned().context("--role needs a value")?;
            }
            "--expires-in-days" => {
                let raw = iter.next().context("--expires-in-days needs a value")?;
                let days: i64 = raw.parse().with_context(|| {
                    format!("--expires-in-days must be a positive integer (got `{raw}`)")
                })?;
                if days <= 0 {
                    bail!("--expires-in-days must be positive (got {days})");
                }
                expires_in_days = Some(days);
            }
            other => bail!("unknown argument `{other}`"),
        }
    }
    let name = name.context("--name is required")?;
    if !crate::rbac::Role::is_valid(&role) {
        bail!("--role must be one of: owner, publisher, reader (got `{role}`)");
    }
    if org_slug.is_none() && role != "owner" {
        bail!(
            "--role only applies to org-scoped tokens; pass --org, or omit --role for an admin token"
        );
    }

    let cfg = Config::from_env()?;
    let db = Database::connect(&cfg.database_url).await?;
    migration::Migrator::up(&db, None).await?;

    let org_id = match &org_slug {
        Some(slug) => Some(find_or_create_org(&db, slug).await?.id),
        None => None,
    };
    let expires_at = expires_in_days.map(|days| chrono::Utc::now() + chrono::Duration::days(days));

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
        role: ActiveValue::Set(role.clone()),
        created_at: ActiveValue::Set(chrono::Utc::now()),
        expires_at: ActiveValue::Set(expires_at),
        revoked_at: ActiveValue::Set(None),
    }
    .insert(&db)
    .await?;

    let scope = match &org_slug {
        Some(slug) => format!("org `{slug}`, role `{role}`"),
        None => "admin (all orgs)".to_string(),
    };
    let expiry = match expires_at {
        Some(at) => format!(", expires {}", at.to_rfc3339()),
        None => String::new(),
    };
    println!("token `{name}` created ({scope}{expiry}); save it now, it is shown exactly once:");
    println!("{plaintext}");
    Ok(())
}

/// `zed-api-server revoke-token --name <name>` (or `--id <uuid>`): mark every
/// matching token revoked so it stops being accepted immediately. Idempotent.
pub async fn revoke_token(args: &[String]) -> Result<()> {
    let mut name = None;
    let mut id = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--name" => name = iter.next().cloned(),
            "--id" => id = iter.next().cloned(),
            other => bail!("unknown argument `{other}`"),
        }
    }
    if name.is_none() && id.is_none() {
        bail!("pass --name <name> or --id <uuid> to identify the token to revoke");
    }

    let cfg = Config::from_env()?;
    let db = Database::connect(&cfg.database_url).await?;
    migration::Migrator::up(&db, None).await?;

    let mut query = token::Entity::find();
    if let Some(name) = &name {
        query = query.filter(token::Column::Name.eq(name.as_str()));
    }
    if let Some(id) = &id {
        let uuid = uuid::Uuid::parse_str(id).context("--id must be a UUID")?;
        query = query.filter(token::Column::Id.eq(uuid));
    }
    let matches = query.all(&db).await?;
    if matches.is_empty() {
        bail!("no token matched");
    }

    let now = chrono::Utc::now();
    let mut revoked = 0;
    for row in matches {
        // Preserve an existing revocation timestamp; revoking is idempotent.
        let already = row.revoked_at.is_some();
        let mut active: token::ActiveModel = row.into();
        if !already {
            active.revoked_at = ActiveValue::Set(Some(now));
            active.update(&db).await?;
            revoked += 1;
        }
    }
    println!("revoked {revoked} token(s)");
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
        created_by_token: ActiveValue::Set(None),
    }
    .insert(db)
    .await?)
}
