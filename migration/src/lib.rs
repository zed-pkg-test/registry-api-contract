pub use sea_orm_migration::prelude::*;

mod m20260723_000001_init;
mod m20260724_000002_version_scheme;
mod m20260724_000003_org_created_by;
mod m20260724_000004_token_role;
mod m20260724_000005_token_lifecycle;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260723_000001_init::Migration),
            Box::new(m20260724_000002_version_scheme::Migration),
            Box::new(m20260724_000003_org_created_by::Migration),
            Box::new(m20260724_000004_token_role::Migration),
        ]
    }
}
