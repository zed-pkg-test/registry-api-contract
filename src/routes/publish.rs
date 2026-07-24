//! The publish pipeline: auth -> validate -> sha256 recompute -> org
//! ownership -> VCS tag verification -> immutability -> store -> record.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderMap, StatusCode};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;
use zed_interfaces::registry::{
    PUBLISH_ARTIFACT_FIELD, PUBLISH_META_FIELD, PublishMeta, PublishResponse,
};

use crate::auth::require_token;
use crate::entities::{org, package, version};
use crate::error::{ApiErr, ApiResult};
use crate::state::AppState;
use crate::storage::artifact_key;
use crate::verify::TagCheck;

pub async fn publish(
    State(state): State<Arc<AppState>>,
    Path((org_slug, name, ver)): Path<(String, String, String)>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> ApiResult<Json<PublishResponse>> {
    let token = require_token(&state.db, &headers).await?;

    let (meta, artifact) = read_multipart(&mut multipart).await?;

    meta.manifest
        .validate()
        .map_err(|e| ApiErr::bad_request("invalid_manifest", e.to_string()))?;
    let m = &meta.manifest.package;
    if m.org != org_slug || m.name != name || m.version != ver {
        return Err(ApiErr::bad_request(
            "manifest_url_mismatch",
            format!(
                "manifest says {}/{}@{}, url says {org_slug}/{name}@{ver}",
                m.org, m.name, m.version
            ),
        ));
    }

    let actual_sha = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&artifact));
    if actual_sha != meta.sha256 {
        return Err(ApiErr::bad_request(
            "sha256_mismatch",
            format!(
                "client declared {}, server computed {actual_sha}",
                meta.sha256
            ),
        ));
    }

    let org_row = org::Entity::find()
        .filter(org::Column::Slug.eq(&org_slug))
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiErr {
            status: StatusCode::NOT_FOUND,
            code: "org_not_found",
            message: format!(
                "org `{org_slug}` does not exist; claim it first with `zed org claim {org_slug}`"
            ),
        })?;
    if let Some(scope) = token.org_id {
        if scope != org_row.id {
            return Err(ApiErr::unauthorized());
        }
    }

    match state
        .verifier
        .verify(m.repository.vcs, &m.repository.url, &meta.vcs_tag)
        .await?
    {
        TagCheck::Missing => {
            return Err(ApiErr::bad_request(
                "tag_not_found",
                format!(
                    "tag `{}` not found on {}; authors must tag the backing repo before publishing",
                    meta.vcs_tag, m.repository.url
                ),
            ));
        }
        TagCheck::Verified { .. } | TagCheck::Skipped => {}
    }

    let pkg = upsert_package(&state, &org_row, &name, &meta).await?;

    let exists = version::Entity::find()
        .filter(version::Column::PackageId.eq(pkg.id))
        .filter(version::Column::Version.eq(&ver))
        .one(&state.db)
        .await?;
    if exists.is_some() {
        return Err(ApiErr::conflict(
            "version_exists",
            format!("{org_slug}/{name}@{ver} is already published; versions are immutable"),
        ));
    }

    let key = artifact_key(&actual_sha, meta.format.extension());
    state
        .store
        .put(&key, artifact.to_vec(), meta.format.content_type())
        .await?;

    version::ActiveModel {
        id: ActiveValue::Set(Uuid::new_v4()),
        package_id: ActiveValue::Set(pkg.id),
        version: ActiveValue::Set(ver.clone()),
        sha256: ActiveValue::Set(actual_sha.clone()),
        size: ActiveValue::Set(artifact.len() as i64),
        format: ActiveValue::Set(meta.format.extension().to_string()),
        vcs_tag: ActiveValue::Set(meta.vcs_tag.clone()),
        vcs_commit: ActiveValue::Set(meta.vcs_commit.clone()),
        artifact_key: ActiveValue::Set(key),
        yanked: ActiveValue::Set(false),
        published_at: ActiveValue::Set(Utc::now()),
    }
    .insert(&state.db)
    .await?;

    tracing::info!(org = %org_slug, name = %name, version = %ver, sha256 = %actual_sha, "published");
    Ok(Json(PublishResponse {
        org: org_slug,
        name,
        version: ver,
        sha256: actual_sha,
    }))
}

async fn read_multipart(multipart: &mut Multipart) -> ApiResult<(PublishMeta, Bytes)> {
    let mut meta: Option<PublishMeta> = None;
    let mut artifact: Option<Bytes> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiErr::bad_request("invalid_multipart", e.to_string()))?
    {
        match field.name() {
            Some(PUBLISH_META_FIELD) => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| ApiErr::bad_request("invalid_meta", e.to_string()))?;
                meta = Some(
                    serde_json::from_str(&text)
                        .map_err(|e| ApiErr::bad_request("invalid_meta", e.to_string()))?,
                );
            }
            Some(PUBLISH_ARTIFACT_FIELD) => {
                artifact = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| ApiErr::bad_request("invalid_artifact", e.to_string()))?,
                );
            }
            _ => {}
        }
    }
    let meta = meta.ok_or_else(|| ApiErr::bad_request("invalid_meta", "missing meta field"))?;
    let artifact = artifact
        .ok_or_else(|| ApiErr::bad_request("invalid_artifact", "missing artifact field"))?;
    Ok((meta, artifact))
}

async fn upsert_package(
    state: &AppState,
    org_row: &org::Model,
    name: &str,
    meta: &PublishMeta,
) -> ApiResult<package::Model> {
    let m = &meta.manifest.package;
    Ok(
        match package::Entity::find()
            .filter(package::Column::OrgId.eq(org_row.id))
            .filter(package::Column::Name.eq(name))
            .one(&state.db)
            .await?
        {
            Some(existing) => {
                let mut active: package::ActiveModel = existing.into();
                active.description = ActiveValue::Set(m.description.clone());
                active.vcs = ActiveValue::Set(m.repository.vcs.to_string());
                active.repo_url = ActiveValue::Set(m.repository.url.clone());
                active.update(&state.db).await?
            }
            None => {
                package::ActiveModel {
                    id: ActiveValue::Set(Uuid::new_v4()),
                    org_id: ActiveValue::Set(org_row.id),
                    name: ActiveValue::Set(name.to_string()),
                    description: ActiveValue::Set(m.description.clone()),
                    vcs: ActiveValue::Set(m.repository.vcs.to_string()),
                    repo_url: ActiveValue::Set(m.repository.url.clone()),
                    created_at: ActiveValue::Set(Utc::now()),
                }
                .insert(&state.db)
                .await?
            }
        },
    )
}
