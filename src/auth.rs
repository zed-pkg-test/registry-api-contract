use axum::http::HeaderMap;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use sha2::{Digest, Sha256};

use crate::entities::token;
use crate::error::{ApiErr, ApiResult};

/// Tokens are stored as sha256 hex; the plaintext is shown exactly once by
/// the `create-token` subcommand.
pub fn hash_token(plaintext: &str) -> String {
    hex::encode(Sha256::digest(plaintext.as_bytes()))
}

pub fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|t| t.trim().to_string())
}

pub async fn require_token(
    db: &DatabaseConnection,
    headers: &HeaderMap,
) -> ApiResult<token::Model> {
    let plaintext = bearer_token(headers).ok_or_else(ApiErr::unauthorized)?;
    token::Entity::find()
        .filter(token::Column::TokenHash.eq(hash_token(&plaintext)))
        .one(db)
        .await?
        .ok_or_else(ApiErr::unauthorized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashing_is_stable_and_hex() {
        let h = hash_token("zpkg_example");
        assert_eq!(h.len(), 64);
        assert_eq!(h, hash_token("zpkg_example"));
        assert_ne!(h, hash_token("zpkg_other"));
    }

    #[test]
    fn bearer_extraction() {
        let mut headers = HeaderMap::new();
        assert!(bearer_token(&headers).is_none());
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer zpkg_abc".parse().unwrap(),
        );
        assert_eq!(bearer_token(&headers).as_deref(), Some("zpkg_abc"));
    }
}
