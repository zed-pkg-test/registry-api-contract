//! Metadata view of the `package_embedding` table. The `embedding` column
//! itself (pgvector `vector(2050)`) is NOT mapped here — SeaORM has no pgvector
//! column type, so writes and similarity search go through raw SQL in
//! `crate::embeddings`. This entity exists for the metadata columns (listing a
//! package's embeddings, existence checks, deletes).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "package_embedding")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub package_id: Uuid,
    pub embedding_model: String,
    pub native_dimensions: i32,
    pub content: String,
    pub content_sha256: String,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::package::Entity",
        from = "Column::PackageId",
        to = "super::package::Column::Id"
    )]
    Package,
}

impl Related<super::package::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Package.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
