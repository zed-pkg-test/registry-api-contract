//! Read an org's audit log (zed-docs issue #7, governance).
//!
//! Owner-only: the trail names which token performed each mutation, so it is
//! not information a `publisher`/`reader` token should be able to enumerate.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::Deserialize;
use zed_interfaces::registry::{AuditAction, AuditEntry, AuditLogResponse};

use crate::auth::require_token;
use crate::entities::audit_log;
use crate::error::ApiResult;
use crate::state::AppState;

use super::find_org;

/// Newest-first page size when the caller doesn't ask, and the hard ceiling.
/// Bounded so a long-lived org cannot be used to force an unbounded response.
const DEFAULT_LIMIT: u64 = 100;
const MAX_LIMIT: u64 = 1000;

#[derive(Debug, Default, Deserialize)]
pub struct AuditQuery {
    limit: Option<u64>,
}

pub async fn get_audit_log(
    State(state): State<Arc<AppState>>,
    Path(org_slug): Path<String>,
    Query(query): Query<AuditQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<AuditLogResponse>> {
    let token = require_token(&state.db, &headers).await?;
    let org_row = find_org(&state, &org_slug).await?;
    crate::rbac::authorize_manage(
        token.org_id,
        crate::rbac::Role::parse(&token.role),
        org_row.id,
    )?;

    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let rows = audit_log::Entity::find()
        .filter(audit_log::Column::OrgId.eq(org_row.id))
        .order_by_desc(audit_log::Column::At)
        .limit(limit)
        .all(&state.db)
        .await?;

    let entries = rows
        .into_iter()
        .map(|r| AuditEntry {
            at: r.at.to_rfc3339(),
            action_kind: AuditAction::parse(&r.action),
            action: r.action,
            subject: r.subject,
            actor_token_name: r.actor_token_name,
            actor_role: r.actor_role,
            detail: r.detail,
        })
        .collect();
    Ok(Json(AuditLogResponse {
        org: org_slug,
        entries,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use sea_orm::{
        ActiveModelTrait, ActiveValue, ConnectOptions, ConnectionTrait, Database,
        DatabaseConnection, Schema,
    };
    use uuid::Uuid;
    use zed_interfaces::registry::AuditAction;

    use crate::auth::hash_token;
    use crate::config::{StorageConfig, TagPolicy};
    use crate::entities::{org, token};
    use crate::storage::ArtifactStore;
    use crate::verify::TagVerifier;

    async fn test_state() -> Arc<AppState> {
        let mut opts = ConnectOptions::new("sqlite::memory:".to_string());
        opts.max_connections(1)
            .min_connections(1)
            .sqlx_logging(false);
        let db: DatabaseConnection = Database::connect(opts).await.unwrap();
        let backend = db.get_database_backend();
        let schema = Schema::new(backend);
        for stmt in [
            schema.create_table_from_entity(org::Entity),
            schema.create_table_from_entity(token::Entity),
            schema.create_table_from_entity(audit_log::Entity),
        ] {
            db.execute(backend.build(&stmt)).await.unwrap();
        }
        let dir = std::env::temp_dir().join(format!("zed-api-audit-test-{}", Uuid::new_v4()));
        Arc::new(AppState {
            db,
            store: ArtifactStore::from_config(&StorageConfig::Local {
                dir: dir.to_string_lossy().to_string(),
            })
            .await
            .unwrap(),
            verifier: TagVerifier::new(TagPolicy::Off),
            public_base_url: "http://localhost:8080".to_string(),
            max_orgs_per_token: 5,
            fiducia: None,
            // Unit tests call handlers directly and must not be throttled.
            rate_limiter: None,
        })
    }

    /// Seed org `acme` plus a token scoped to it with `role`; returns
    /// (org_id, token plaintext).
    async fn seed(state: &AppState, role: &str) -> (Uuid, String) {
        let org_id = Uuid::new_v4();
        org::ActiveModel {
            id: ActiveValue::Set(org_id),
            slug: ActiveValue::Set("acme".to_string()),
            created_at: ActiveValue::Set(Utc::now()),
            created_by_token: ActiveValue::Set(None),
        }
        .insert(&state.db)
        .await
        .unwrap();
        let plaintext = format!("zpkg_{role}_{}", Uuid::new_v4().simple());
        token::ActiveModel {
            id: ActiveValue::Set(Uuid::new_v4()),
            name: ActiveValue::Set(format!("{role}-token")),
            token_hash: ActiveValue::Set(hash_token(&plaintext)),
            org_id: ActiveValue::Set(Some(org_id)),
            role: ActiveValue::Set(role.to_string()),
            created_at: ActiveValue::Set(Utc::now()),
            expires_at: ActiveValue::Set(None),
            revoked_at: ActiveValue::Set(None),
        }
        .insert(&state.db)
        .await
        .unwrap();
        (org_id, plaintext)
    }

    fn bearer(plaintext: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {plaintext}").parse().unwrap(),
        );
        headers
    }

    fn call(
        state: &Arc<AppState>,
        headers: HeaderMap,
        limit: Option<u64>,
    ) -> impl std::future::Future<Output = ApiResult<Json<AuditLogResponse>>> {
        get_audit_log(
            State(state.clone()),
            Path("acme".to_string()),
            Query(AuditQuery { limit }),
            headers,
        )
    }

    /// The audit log names who acted, so only an `owner` (or admin) may read it.
    #[tokio::test]
    async fn audit_log_is_owner_only() {
        for role in ["publisher", "reader"] {
            let state = test_state().await;
            let (_org, plaintext) = seed(&state, role).await;
            let err = call(&state, bearer(&plaintext), None)
                .await
                .expect_err("non-owner must not read the audit log");
            assert_eq!(err.code, "insufficient_role", "role {role}");
        }
        // An owner can.
        let state = test_state().await;
        let (_org, owner) = seed(&state, "owner").await;
        assert!(call(&state, bearer(&owner), None).await.is_ok());
    }

    #[tokio::test]
    async fn missing_token_is_unauthorized() {
        let state = test_state().await;
        let _ = seed(&state, "owner").await;
        let err = call(&state, HeaderMap::new(), None)
            .await
            .expect_err("no bearer token must be rejected");
        assert_eq!(err.code, "unauthorized");
    }

    /// Entries come back newest-first, carry the acting token's identity, and
    /// parse into a known action kind.
    #[tokio::test]
    async fn entries_are_newest_first_and_name_the_actor() {
        let state = test_state().await;
        let (org_id, owner) = seed(&state, "owner").await;
        let actor = token::Entity::find().one(&state.db).await.unwrap().unwrap();

        for (i, (action, subject)) in [
            (AuditAction::Publish, "acme/http-kit@1.0.0"),
            (AuditAction::Yank, "acme/http-kit@1.0.0"),
            (AuditAction::Unyank, "acme/http-kit@1.0.0"),
        ]
        .into_iter()
        .enumerate()
        {
            // Distinct, increasing timestamps so ordering is unambiguous.
            audit_log::ActiveModel {
                id: ActiveValue::Set(Uuid::new_v4()),
                org_id: ActiveValue::Set(org_id),
                at: ActiveValue::Set(Utc::now() + chrono::Duration::seconds(i as i64)),
                action: ActiveValue::Set(action.as_str().to_string()),
                subject: ActiveValue::Set(subject.to_string()),
                actor_token_id: ActiveValue::Set(Some(actor.id)),
                actor_token_name: ActiveValue::Set(actor.name.clone()),
                actor_role: ActiveValue::Set("owner".to_string()),
                detail: ActiveValue::Set(None),
            }
            .insert(&state.db)
            .await
            .unwrap();
        }

        let resp = call(&state, bearer(&owner), None).await.unwrap().0;
        assert_eq!(resp.org, "acme");
        let kinds: Vec<_> = resp.entries.iter().map(|e| e.action_kind).collect();
        assert_eq!(
            kinds,
            vec![
                Some(AuditAction::Unyank),
                Some(AuditAction::Yank),
                Some(AuditAction::Publish)
            ],
            "newest first"
        );
        assert_eq!(resp.entries[0].actor_token_name, "owner-token");
        assert_eq!(resp.entries[0].actor_role, "owner");
    }

    /// `limit` is honored and clamped, so a huge org can't force a huge body.
    #[tokio::test]
    async fn limit_is_honored_and_clamped() {
        let state = test_state().await;
        let (org_id, owner) = seed(&state, "owner").await;
        for i in 0..5 {
            audit_log::ActiveModel {
                id: ActiveValue::Set(Uuid::new_v4()),
                org_id: ActiveValue::Set(org_id),
                at: ActiveValue::Set(Utc::now() + chrono::Duration::seconds(i)),
                action: ActiveValue::Set("publish".to_string()),
                subject: ActiveValue::Set(format!("acme/p@1.0.{i}")),
                actor_token_id: ActiveValue::Set(None),
                actor_token_name: ActiveValue::Set("t".to_string()),
                actor_role: ActiveValue::Set("owner".to_string()),
                detail: ActiveValue::Set(None),
            }
            .insert(&state.db)
            .await
            .unwrap();
        }
        let two = call(&state, bearer(&owner), Some(2)).await.unwrap().0;
        assert_eq!(two.entries.len(), 2);
        // 0 clamps up to 1 rather than returning nothing or erroring.
        let zero = call(&state, bearer(&owner), Some(0)).await.unwrap().0;
        assert_eq!(zero.entries.len(), 1);
        // Absurd limits clamp to MAX_LIMIT instead of being rejected.
        let huge = call(&state, bearer(&owner), Some(u64::MAX))
            .await
            .unwrap()
            .0;
        assert_eq!(huge.entries.len(), 5);
    }

    /// An unrecognized stored action still reads back (forward compatibility):
    /// the raw string survives and `action_kind` is simply absent.
    #[tokio::test]
    async fn unknown_action_strings_survive_a_read() {
        let state = test_state().await;
        let (org_id, owner) = seed(&state, "owner").await;
        audit_log::ActiveModel {
            id: ActiveValue::Set(Uuid::new_v4()),
            org_id: ActiveValue::Set(org_id),
            at: ActiveValue::Set(Utc::now()),
            action: ActiveValue::Set("transfer_ownership".to_string()),
            subject: ActiveValue::Set("acme".to_string()),
            actor_token_id: ActiveValue::Set(None),
            actor_token_name: ActiveValue::Set("t".to_string()),
            actor_role: ActiveValue::Set("admin".to_string()),
            detail: ActiveValue::Set(None),
        }
        .insert(&state.db)
        .await
        .unwrap();
        let resp = call(&state, bearer(&owner), None).await.unwrap().0;
        assert_eq!(resp.entries[0].action, "transfer_ownership");
        assert_eq!(resp.entries[0].action_kind, None);
    }
}
