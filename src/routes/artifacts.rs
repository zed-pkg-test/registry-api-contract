//! Artifact download (by sha256) and unpkg-style single-file serving.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::entities::version;
use crate::error::{ApiErr, ApiResult};
use crate::files;
use crate::state::AppState;
use crate::storage::Download;

use super::{artifact_format, find_org, find_package};

const IMMUTABLE: &str = "public, max-age=31536000, immutable";

pub async fn get_artifact(
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
                (
                    header::CONTENT_TYPE,
                    artifact_format(&row.format).content_type().to_string(),
                ),
                (header::CACHE_CONTROL, IMMUTABLE.to_string()),
            ],
            bytes,
        )
            .into_response()),
    }
}

pub async fn get_file(
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
            (header::CACHE_CONTROL, IMMUTABLE.to_string()),
        ],
        file,
    )
        .into_response())
}
