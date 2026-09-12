use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "token")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub name: String,
    #[sea_orm(unique)]
    pub token_hash: String,
    /// Scope: Some(org) limits the token to that org; None is an admin token.
    pub org_id: Option<Uuid>,
    /// RBAC role within the org scope: `owner`, `publisher`, or `reader`
    /// (zed-docs issue #7). Admin tokens (no org scope) are owner-equivalent
    /// everywhere.
    #[sea_orm(default_value = "owner")]
    pub role: String,
    pub created_at: DateTimeUtc,
    /// When the token stops being accepted (NULL = never expires).
    pub expires_at: Option<DateTimeUtc>,
    /// When the token was revoked (NULL = live). A revoked token is rejected
    /// regardless of expiry.
    pub revoked_at: Option<DateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
