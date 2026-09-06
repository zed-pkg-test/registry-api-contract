//! Add `role` to `token` (zed-docs issue #7, governance/RBAC): the capability
//! a token grants within its org scope — `owner`, `publisher`, or `reader`.
//! Existing tokens default to `owner`, preserving today's behavior.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Token::Table)
                    .add_column(
                        ColumnDef::new(Token::Role)
                            .string()
                            .not_null()
                            .default("owner"),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Token::Table)
                    .drop_column(Token::Role)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Token {
    Table,
    Role,
}
