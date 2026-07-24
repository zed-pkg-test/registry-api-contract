//! Add `created_by_token` to `org`: which token claimed the namespace, so
//! per-token claim quotas can stop namespace squatting. Nullable (pre-existing
//! orgs and CLI-minted ones have no claimer); SET NULL if the token goes away.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Org::Table)
                    .add_column(ColumnDef::new(Org::CreatedByToken).uuid())
                    .to_owned(),
            )
            .await?;
        manager
            .create_foreign_key(
                ForeignKey::create()
                    .name("fk_org_created_by_token")
                    .from(Org::Table, Org::CreatedByToken)
                    .to(Token::Table, Token::Id)
                    .on_delete(ForeignKeyAction::SetNull)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_foreign_key(
                ForeignKey::drop()
                    .name("fk_org_created_by_token")
                    .table(Org::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Org::Table)
                    .drop_column(Org::CreatedByToken)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Org {
    Table,
    CreatedByToken,
}

#[derive(DeriveIden)]
enum Token {
    Table,
    Id,
}
