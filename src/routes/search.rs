//! Package search across name and description.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use sea_orm::sea_query::LikeExpr;
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::Deserialize;
use zed_interfaces::registry::{PackageSummary, SearchResponse};

use crate::entities::{org, package, version};
use crate::error::ApiResult;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct SearchParams {
    #[serde(default)]
    q: String,
}

/// Neutralize LIKE wildcards in user queries so `%`/`_` match literally
/// (backslash first, so escapes are not themselves re-escaped). Used with an
/// explicit `ESCAPE '\'` clause; the pattern stays a bind parameter.
fn escape_like(q: &str) -> String {
    q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

fn contains_escaped(q: &str) -> LikeExpr {
    LikeExpr::new(format!("%{}%", escape_like(q))).escape('\\')
}

pub async fn search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> ApiResult<Json<SearchResponse>> {
    let rows = package::Entity::find()
        .filter(
            Condition::any()
                .add(package::Column::Name.like(contains_escaped(&params.q)))
                .add(package::Column::Description.like(contains_escaped(&params.q))),
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
