use sea_orm::entity::prelude::*;

/// An append-only audit record of one mutation of published state
/// (zed-docs issue #7). Rows are never updated or deleted by the server.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "audit_log")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub org_id: Uuid,
    pub at: DateTimeUtc,
    /// `publish` | `yank` | `unyank` | `org_claim`.
    pub action: String,
    /// What was acted on, e.g. `acme/http-kit@1.2.0`.
    pub subject: String,
    /// The acting token's id. Intentionally not a foreign key: the trail must
    /// survive the token being deleted.
    pub actor_token_id: Option<Uuid>,
    /// Denormalized so the record stays readable after token deletion.
    pub actor_token_name: String,
    /// `owner` | `publisher` | `reader`, or `admin` for unscoped tokens.
    pub actor_role: String,
    pub detail: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::org::Entity",
        from = "Column::OrgId",
        to = "super::org::Column::Id"
    )]
    Org,
}

impl Related<super::org::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Org.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
