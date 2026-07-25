use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use aws_sdk_s3::presigning::PresigningConfig;

use crate::config::StorageConfig;

/// Artifact storage: local disk for dev/self-host, or any S3-compatible
/// endpoint (AWS S3, Cloudflare R2, MinIO) for production.
pub enum ArtifactStore {
    Local {
        dir: PathBuf,
    },
    S3 {
        client: aws_sdk_s3::Client,
        bucket: String,
    },
}

/// How a download should be served to the client.
pub enum Download {
    /// 302 to a presigned URL (S3/R2).
    Redirect(String),
    /// Stream the bytes directly (local backend).
    Bytes(Vec<u8>),
}

impl ArtifactStore {
    pub async fn from_config(config: &StorageConfig) -> Result<Self> {
        match config {
            StorageConfig::Local { dir } => {
                let dir = PathBuf::from(dir);
                tokio::fs::create_dir_all(&dir).await?;
                Ok(Self::Local { dir })
            }
            StorageConfig::S3 {
                bucket,
                endpoint_url,
                region,
                force_path_style,
            } => {
                let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest())
                    .region(aws_config::Region::new(region.clone()));
                if let Some(endpoint) = endpoint_url {
                    loader = loader.endpoint_url(endpoint);
                }
                let shared = loader.load().await;
                let s3_config = aws_sdk_s3::config::Builder::from(&shared)
                    .force_path_style(*force_path_style)
                    .build();
                Ok(Self::S3 {
                    client: aws_sdk_s3::Client::from_conf(s3_config),
                    bucket: bucket.clone(),
                })
            }
        }
    }

    fn local_path(dir: &PathBuf, key: &str) -> PathBuf {
        // Keys are server-generated (`artifacts/<sha>.<ext>`), never user input.
        dir.join(key)
    }

    /// Takes `Bytes` rather than `Vec<u8>`: the artifact arrives from the
    /// multipart reader as `Bytes`, and a `Vec` signature forced a second full
    /// copy of every upload (peak ~2x the artifact size per in-flight publish).
    /// Both backends consume `Bytes` without copying.
    pub async fn put(&self, key: &str, bytes: Bytes, content_type: &str) -> Result<()> {
        match self {
            Self::Local { dir } => {
                let path = Self::local_path(dir, key);
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(path, bytes).await?;
                Ok(())
            }
            Self::S3 { client, bucket } => {
                client
                    .put_object()
                    .bucket(bucket)
                    .key(key)
                    .content_type(content_type)
                    .body(bytes.into())
                    .send()
                    .await
                    .context("s3 put_object failed")?;
                Ok(())
            }
        }
    }

    pub async fn download(&self, key: &str) -> Result<Download> {
        match self {
            Self::Local { dir } => {
                let bytes = tokio::fs::read(Self::local_path(dir, key)).await?;
                Ok(Download::Bytes(bytes))
            }
            Self::S3 { client, bucket } => {
                let presigned = client
                    .get_object()
                    .bucket(bucket)
                    .key(key)
                    .presigned(PresigningConfig::expires_in(Duration::from_secs(600))?)
                    .await
                    .context("s3 presign failed")?;
                Ok(Download::Redirect(presigned.uri().to_string()))
            }
        }
    }

    /// Remove a stored artifact (best-effort cleanup paths).
    pub async fn delete(&self, key: &str) -> Result<()> {
        match self {
            Self::Local { dir } => {
                tokio::fs::remove_file(Self::local_path(dir, key)).await?;
                Ok(())
            }
            Self::S3 { client, bucket } => {
                client
                    .delete_object()
                    .bucket(bucket)
                    .key(key)
                    .send()
                    .await
                    .context("s3 delete_object failed")?;
                Ok(())
            }
        }
    }

    /// Full artifact bytes regardless of backend (for /v1/files extraction).
    pub async fn get_bytes(&self, key: &str) -> Result<Vec<u8>> {
        match self {
            Self::Local { dir } => Ok(tokio::fs::read(Self::local_path(dir, key)).await?),
            Self::S3 { client, bucket } => {
                let object = client
                    .get_object()
                    .bucket(bucket)
                    .key(key)
                    .send()
                    .await
                    .context("s3 get_object failed")?;
                Ok(object.body.collect().await?.into_bytes().to_vec())
            }
        }
    }
}

pub fn artifact_key(sha256: &str, extension: &str) -> String {
    format!("artifacts/{sha256}.{extension}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_sha_addressed() {
        assert_eq!(artifact_key("abc123", "tar.gz"), "artifacts/abc123.tar.gz");
    }
}
