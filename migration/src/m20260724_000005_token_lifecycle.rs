//! Token lifecycle (M2): a leaked token was valid forever with no way to kill
//! it short of a manual DB delete. Add nullable `expires_at` and `revoked_at`
//! so tokens can carry a TTL and be revoked; `require_token` rejects a token
//! that is revoked or past expiry. Both NULL preserves today's behavior
//! (non-expiring, live), so existing tokens are unaffected.

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
                    .add_column(ColumnDef::new(Token::ExpiresAt).timestamp_with_time_zone().null())
                    .add_column(ColumnDef::new(Token::RevokedAt).timestamp_with_time_zone().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Token::Table)
                    .drop_column(Token::ExpiresAt)
                    .drop_column(Token::RevokedAt)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Token {
    Table,
    ExpiresAt,
    RevokedAt,
}
