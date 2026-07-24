pub use sea_orm_migration::prelude::*;

mod m20260723_000001_init;
mod m20260724_000002_version_scheme;
mod m20260724_000003_token_role;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260723_000001_init::Migration),
            Box::new(m20260724_000002_version_scheme::Migration),
            Box::new(m20260724_000003_token_role::Migration),
        ]
    }
}
