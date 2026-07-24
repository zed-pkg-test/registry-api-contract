//! Namespace claiming.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, SqlErr,
};
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
    // Squatting quota: org-scoped tokens may only claim a bounded number of
    // namespaces; admin tokens (org_id = None) are exempt.
    if token.org_id.is_some() {
        let claimed = org::Entity::find()
            .filter(org::Column::CreatedByToken.eq(token.id))
            .count(&state.db)
            .await?;
        if claimed >= state.max_orgs_per_token {
            return Err(ApiErr {
                status: StatusCode::FORBIDDEN,
                code: "org_quota_exceeded",
                message: format!(
                    "this token has already claimed {claimed} orgs (limit {}); contact the registry operator",
                    state.max_orgs_per_token
                ),
            });
        }
    }
    org::ActiveModel {
        id: ActiveValue::Set(Uuid::new_v4()),
        slug: ActiveValue::Set(request.slug.clone()),
        created_at: ActiveValue::Set(Utc::now()),
        created_by_token: ActiveValue::Set(Some(token.id)),
    }
    .insert(&state.db)
    .await?;
    Ok(Json(ClaimOrgResponse {
        slug: request.slug,
        created: true,
    }))
}
