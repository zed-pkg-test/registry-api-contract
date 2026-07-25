//! Audit trail for mutations of published state (zed-docs issue #7).
//!
//! Every publish, yank/unyank, and org claim appends one row naming the token
//! that acted. Reads are not audited: the log answers "who *changed* what",
//! and auditing installs would bury that signal under ordinary traffic.
//!
//! **Recording is best-effort by design.** [`record`] is called *after* the
//! mutation has already committed, so a failed audit write must not fail the
//! request — reporting an error would tell the client its publish failed when
//! it actually succeeded, which is worse than a gap in the log (it provokes
//! retries against an immutable version). Failures are logged at `warn` with
//! everything needed to reconstruct the entry by hand.

use sea_orm::{ActiveModelTrait, ActiveValue, DatabaseConnection};
use uuid::Uuid;
use zed_interfaces::registry::AuditAction;

use crate::entities::{audit_log, token};

/// The role string recorded for an unscoped (admin) token, which has no org
/// role of its own but is owner-equivalent everywhere.
pub const ADMIN_ROLE: &str = "admin";

/// The role label to record for `token`.
pub fn actor_role(token: &token::Model) -> &str {
    if token.org_id.is_none() {
        ADMIN_ROLE
    } else {
        &token.role
    }
}

/// Append one audit record. Never returns an error: see the module note on why
/// a failed write must not fail the surrounding request.
pub async fn record(
    db: &DatabaseConnection,
    org_id: Uuid,
    token: &token::Model,
    action: AuditAction,
    subject: impl Into<String>,
    detail: Option<String>,
) {
    let subject = subject.into();
    let entry = audit_log::ActiveModel {
        id: ActiveValue::Set(Uuid::new_v4()),
        org_id: ActiveValue::Set(org_id),
        at: ActiveValue::Set(chrono::Utc::now()),
        action: ActiveValue::Set(action.as_str().to_string()),
        subject: ActiveValue::Set(subject.clone()),
        actor_token_id: ActiveValue::Set(Some(token.id)),
        actor_token_name: ActiveValue::Set(token.name.clone()),
        actor_role: ActiveValue::Set(actor_role(token).to_string()),
        detail: ActiveValue::Set(detail.clone()),
    };
    if let Err(error) = entry.insert(db).await {
        // Loud, and complete enough to reconstruct the row by hand.
        tracing::warn!(
            %error,
            action = action.as_str(),
            subject = %subject,
            actor_token = %token.name,
            detail = ?detail,
            "failed to append audit record; the mutation itself succeeded"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn model(org_id: Option<Uuid>, role: &str) -> token::Model {
        token::Model {
            id: Uuid::new_v4(),
            name: "t".to_string(),
            token_hash: "h".to_string(),
            org_id,
            role: role.to_string(),
            created_at: Utc::now(),
            expires_at: None,
            revoked_at: None,
        }
    }

    /// An unscoped token is recorded as `admin`; a scoped one keeps its role.
    #[test]
    fn admin_tokens_are_labelled_admin() {
        assert_eq!(actor_role(&model(None, "owner")), ADMIN_ROLE);
        assert_eq!(
            actor_role(&model(Some(Uuid::new_v4()), "publisher")),
            "publisher"
        );
    }
}
