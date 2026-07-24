use sea_orm::DatabaseConnection;

use crate::storage::ArtifactStore;
use crate::verify::TagVerifier;

pub struct AppState {
    pub db: DatabaseConnection,
    pub store: ArtifactStore,
    pub verifier: TagVerifier,
    pub public_base_url: String,
}
