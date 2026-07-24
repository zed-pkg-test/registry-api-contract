//! Yank (or restore) a published version. Yanked versions stay downloadable
//! for existing lockfiles but are hidden from resolution and search.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use sea_orm::{ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, QueryFilter};
use zed_interfaces::registry::{YankRequest, YankResponse};

use crate::auth::require_token;
use crate::entities::version;
use crate::error::{ApiErr, ApiResult};
use crate::state::AppState;

use super::{find_org, find_package};

pub async fn yank(
    State(state): State<Arc<AppState>>,
    Path((org_slug, name, ver)): Path<(String, String, String)>,
    headers: HeaderMap,
    Json(request): Json<YankRequest>,
) -> ApiResult<Json<YankResponse>> {
    let token = require_token(&state.db, &headers).await?;
    let org_row = find_org(&state, &org_slug).await?;
    // Same rule as publish: admin tokens (no scope) or tokens scoped to this org.
    if let Some(scope) = token.org_id {
        if scope != org_row.id {
            return Err(ApiErr::unauthorized());
        }
    }
    let pkg = find_package(&state, &org_row, &name).await?;
    let row = version::Entity::find()
        .filter(version::Column::PackageId.eq(pkg.id))
        .filter(version::Column::Version.eq(&ver))
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiErr::not_found("version"))?;

    let mut active: version::ActiveModel = row.into();
    active.yanked = ActiveValue::Set(request.yanked);
    let updated = active.update(&state.db).await?;

    tracing::info!(org = %org_slug, name = %name, version = %ver, yanked = updated.yanked, "yank state changed");
    Ok(Json(YankResponse {
        org: org_slug,
        name,
        version: ver,
        yanked: updated.yanked,
    }))
}
