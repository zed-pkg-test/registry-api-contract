//! Add `version_scheme` to `package` (zed-docs issue #3): how a package's
//! versions are interpreted (semver by default, else calver/opaque). Existing
//! rows default to `semver`, which is the historical behavior.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Package::Table)
                    .add_column(
                        ColumnDef::new(Package::VersionScheme)
                            .string()
                            .not_null()
                            .default("semver"),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Package::Table)
                    .drop_column(Package::VersionScheme)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Package {
    Table,
    VersionScheme,
}
