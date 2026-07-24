//! Namespace claiming.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;
use zed_interfaces::manifest::is_slug;
use zed_interfaces::registry::{ClaimOrgRequest, ClaimOrgResponse};

use crate::auth::require_token;
use crate::entities::org;
use crate::error::{ApiErr, ApiResult};
use crate::state::AppState;

pub async fn claim_org(
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
