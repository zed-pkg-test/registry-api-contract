use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use serde::Deserialize;
use uuid::Uuid;
use zed_interfaces::manifest::is_slug;
use zed_interfaces::registry::{
    ClaimOrgRequest, ClaimOrgResponse, PUBLISH_ARTIFACT_FIELD, PUBLISH_META_FIELD, PackageMetadata,
    PackageSummary, PublishMeta, PublishResponse, SearchResponse, VersionMetadata,
};

use crate::auth::require_token;
use crate::entities::{org, package, version};
use crate::error::{ApiErr, ApiResult};
use crate::files;
use crate::state::AppState;
use crate::storage::{Download, artifact_key};
use crate::verify::TagCheck;

pub const ROUTE_PACKAGE: &str = "/v1/packages/{org}/{name}";
pub const ROUTE_VERSION: &str = "/v1/packages/{org}/{name}/versions/{version}";
pub const ROUTE_ARTIFACT: &str = "/v1/artifacts/{sha256}";
pub const ROUTE_SEARCH: &str = "/v1/search";
pub const ROUTE_ORGS: &str = "/v1/orgs";
pub const ROUTE_FILES: &str = "/v1/files/{org}/{name}/{version}/{*path}";

pub fn router(state: Arc<AppState>, max_artifact_bytes: usize) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route(ROUTE_PACKAGE, get(get_package))
        .route(ROUTE_VERSION, get(get_version).put(publish))
        .route(ROUTE_ARTIFACT, get(get_artifact))
        .route(ROUTE_SEARCH, get(search))
        .route(ROUTE_ORGS, post(claim_org))
        .route(ROUTE_FILES, get(get_file))
        .layer(DefaultBodyLimit::max(max_artifact_bytes))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

async fn healthz(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let db_ok = state.db.ping().await.is_ok();
    Json(serde_json::json!({ "ok": true, "db": db_ok }))
}

async fn find_org(state: &AppState, slug: &str) -> ApiResult<org::Model> {
    org::Entity::find()
        .filter(org::Column::Slug.eq(slug))
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiErr::not_found("org"))
}

async fn find_package(
    state: &AppState,
    org_row: &org::Model,
    name: &str,
) -> ApiResult<package::Model> {
    package::Entity::find()
        .filter(package::Column::OrgId.eq(org_row.id))
        .filter(package::Column::Name.eq(name))
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiErr::not_found("package"))
}

fn sort_versions_desc(versions: &mut [String]) {
    versions.sort_by(|a, b| {
        let pa = semver::Version::parse(a).ok();
        let pb = semver::Version::parse(b).ok();
        pb.cmp(&pa)
    });
}

fn version_metadata(
    state: &AppState,
    org: &str,
    name: &str,
    row: &version::Model,
) -> VersionMetadata {
    VersionMetadata {
        org: org.to_string(),
        name: name.to_string(),
        version: row.version.clone(),
        sha256: row.sha256.clone(),
        size: row.size as u64,
        format: serde_json::from_value(serde_json::Value::String(row.format.clone()))
            .unwrap_or_default(),
        vcs_tag: row.vcs_tag.clone(),
        vcs_commit: row.vcs_commit.clone(),
        download_url: format!(
            "{}{}",
            state.public_base_url,
            zed_interfaces::registry::artifact_path(&row.sha256)
        ),
        published_at: row.published_at.to_rfc3339(),
        yanked: row.yanked,
    }
}

async fn get_package(
    State(state): State<Arc<AppState>>,
    Path((org_slug, name)): Path<(String, String)>,
) -> ApiResult<Json<PackageMetadata>> {
    let org_row = find_org(&state, &org_slug).await?;
    let pkg = find_package(&state, &org_row, &name).await?;
    let rows = version::Entity::find()
        .filter(version::Column::PackageId.eq(pkg.id))
        .filter(version::Column::Yanked.eq(false))
        .all(&state.db)
        .await?;
    let mut versions: Vec<String> = rows.iter().map(|r| r.version.clone()).collect();
    sort_versions_desc(&mut versions);
    Ok(Json(PackageMetadata {
        org: org_slug,
        name,
        description: pkg.description,
        vcs: pkg.vcs.parse().unwrap_or_default(),
        repo_url: pkg.repo_url,
        latest: versions.first().cloned(),
        versions,
    }))
}

async fn get_version(
    State(state): State<Arc<AppState>>,
    Path((org_slug, name, ver)): Path<(String, String, String)>,
) -> ApiResult<Json<VersionMetadata>> {
    let org_row = find_org(&state, &org_slug).await?;
    let pkg = find_package(&state, &org_row, &name).await?;
    let row = version::Entity::find()
        .filter(version::Column::PackageId.eq(pkg.id))
        .filter(version::Column::Version.eq(&ver))
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiErr::not_found("version"))?;
    Ok(Json(version_metadata(&state, &org_slug, &name, &row)))
}

async fn publish(
    State(state): State<Arc<AppState>>,
    Path((org_slug, name, ver)): Path<(String, String, String)>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> ApiResult<Json<PublishResponse>> {
    let token = require_token(&state.db, &headers).await?;

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

    let pkg = match package::Entity::find()
        .filter(package::Column::OrgId.eq(org_row.id))
        .filter(package::Column::Name.eq(&name))
        .one(&state.db)
        .await?
    {
        Some(existing) => {
            let mut active: package::ActiveModel = existing.clone().into();
            active.description = ActiveValue::Set(m.description.clone());
            active.vcs = ActiveValue::Set(m.repository.vcs.to_string());
            active.repo_url = ActiveValue::Set(m.repository.url.clone());
            active.update(&state.db).await?
        }
        None => {
            package::ActiveModel {
                id: ActiveValue::Set(Uuid::new_v4()),
                org_id: ActiveValue::Set(org_row.id),
                name: ActiveValue::Set(name.clone()),
                description: ActiveValue::Set(m.description.clone()),
                vcs: ActiveValue::Set(m.repository.vcs.to_string()),
                repo_url: ActiveValue::Set(m.repository.url.clone()),
                created_at: ActiveValue::Set(Utc::now()),
            }
            .insert(&state.db)
            .await?
        }
    };

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

async fn get_artifact(
    State(state): State<Arc<AppState>>,
    Path(sha256): Path<String>,
) -> ApiResult<Response> {
    let row = version::Entity::find()
        .filter(version::Column::Sha256.eq(&sha256))
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiErr::not_found("artifact"))?;
    match state.store.download(&row.artifact_key).await? {
        Download::Redirect(url) => {
            Ok((StatusCode::FOUND, [(header::LOCATION, url)]).into_response())
        }
        Download::Bytes(bytes) => Ok((
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/gzip".to_string()),
                (
                    header::CACHE_CONTROL,
                    "public, max-age=31536000, immutable".to_string(),
                ),
            ],
            bytes,
        )
            .into_response()),
    }
}

#[derive(Deserialize)]
struct SearchParams {
    #[serde(default)]
    q: String,
}

async fn search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> ApiResult<Json<SearchResponse>> {
    let rows = package::Entity::find()
        .filter(
            Condition::any()
                .add(package::Column::Name.contains(&params.q))
                .add(package::Column::Description.contains(&params.q)),
        )
        .find_also_related(org::Entity)
        .limit(50)
        .all(&state.db)
        .await?;
    let mut items = Vec::with_capacity(rows.len());
    for (pkg, org_row) in rows {
        let Some(org_row) = org_row else { continue };
        let latest = version::Entity::find()
            .filter(version::Column::PackageId.eq(pkg.id))
            .filter(version::Column::Yanked.eq(false))
            .order_by_desc(version::Column::PublishedAt)
            .one(&state.db)
            .await?
            .map(|v| v.version);
        items.push(PackageSummary {
            org: org_row.slug,
            name: pkg.name,
            description: pkg.description,
            latest,
        });
    }
    Ok(Json(SearchResponse {
        query: params.q,
        items,
    }))
}

async fn claim_org(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ClaimOrgRequest>,
) -> ApiResult<Json<ClaimOrgResponse>> {
    let token = require_token(&state.db, &headers).await?;
    if !is_slug(&request.slug) {
        return Err(ApiErr::bad_request(
            "invalid_slug",
            "org slugs are lowercase letters, digits, and hyphens",
        ));
    }
    if let Some(existing) = org::Entity::find()
        .filter(org::Column::Slug.eq(&request.slug))
        .one(&state.db)
        .await?
    {
        if token.org_id == Some(existing.id) {
            return Ok(Json(ClaimOrgResponse {
                slug: request.slug,
                created: false,
            }));
        }
        return Err(ApiErr::conflict(
            "org_taken",
            format!("org `{}` is already claimed", request.slug),
        ));
    }
    org::ActiveModel {
        id: ActiveValue::Set(Uuid::new_v4()),
        slug: ActiveValue::Set(request.slug.clone()),
        created_at: ActiveValue::Set(Utc::now()),
    }
    .insert(&state.db)
    .await?;
    Ok(Json(ClaimOrgResponse {
        slug: request.slug,
        created: true,
    }))
}

async fn get_file(
    State(state): State<Arc<AppState>>,
    Path((org_slug, name, ver, path)): Path<(String, String, String, String)>,
) -> ApiResult<Response> {
    let org_row = find_org(&state, &org_slug).await?;
    let pkg = find_package(&state, &org_row, &name).await?;
    let row = version::Entity::find()
        .filter(version::Column::PackageId.eq(pkg.id))
        .filter(version::Column::Version.eq(&ver))
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiErr::not_found("version"))?;
    let archive = state.store.get_bytes(&row.artifact_key).await?;
    let file = files::extract_file(&archive, &path)
        .map_err(anyhow::Error::from)?
        .ok_or_else(|| ApiErr::not_found("file"))?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, files::mime_for(&path).to_string()),
            (
                header::CACHE_CONTROL,
                "public, max-age=31536000, immutable".to_string(),
            ),
        ],
        file,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TagPolicy;
    use crate::storage::ArtifactStore;
    use crate::verify::TagVerifier;
    use tower::util::ServiceExt;

    /// Route patterns must line up with the URL helpers every client uses.
    #[test]
    fn routes_match_contract_paths() {
        let fill = |pattern: &str| {
            pattern
                .replace("{org}", "acme")
                .replace("{name}", "http-kit")
                .replace("{version}", "1.2.0")
                .replace("{sha256}", "abc")
                .replace("{*path}", "dist/style.css")
        };
        use zed_interfaces::registry as r;
        assert_eq!(fill(ROUTE_PACKAGE), r::package_path("acme", "http-kit"));
        assert_eq!(
            fill(ROUTE_VERSION),
            r::version_path("acme", "http-kit", "1.2.0")
        );
        assert_eq!(fill(ROUTE_ARTIFACT), r::artifact_path("abc"));
        assert_eq!(ROUTE_SEARCH, r::search_path());
        assert_eq!(ROUTE_ORGS, r::orgs_path());
        assert_eq!(
            fill(ROUTE_FILES),
            r::file_path("acme", "http-kit", "1.2.0", "dist/style.css")
        );
    }

    #[tokio::test]
    async fn healthz_works_without_a_database() {
        let dir = std::env::temp_dir().join("zed-api-test-store");
        let state = Arc::new(AppState {
            db: sea_orm::DatabaseConnection::Disconnected,
            store: ArtifactStore::from_config(&crate::config::StorageConfig::Local {
                dir: dir.to_string_lossy().to_string(),
            })
            .await
            .unwrap(),
            verifier: TagVerifier::new(TagPolicy::Off),
            public_base_url: "http://localhost:8080".to_string(),
        });
        let app = router(state, 1024 * 1024);
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/healthz")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
