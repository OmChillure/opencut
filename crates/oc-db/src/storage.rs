use aws_config::BehaviorVersion;
use aws_sdk_s3::Client;
use aws_sdk_s3::config::{
    Credentials, Region, RequestChecksumCalculation, ResponseChecksumValidation,
};
use aws_sdk_s3::presigning::PresigningConfig;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("missing env var {0}")]
    MissingEnv(&'static str),
    #[error("presign: {0}")]
    Presign(String),
    #[error("{0}")]
    Sdk(String),
}

impl From<aws_sdk_s3::Error> for StorageError {
    fn from(value: aws_sdk_s3::Error) -> Self {
        Self::Sdk(explain_store(&value))
    }
}

/// Cloudflare answers a signed request with AccessDenied when the R2 token
/// cannot write the bucket. The SDK otherwise prints "unhandled error".
fn explain_store(err: &aws_sdk_s3::Error) -> String {
    let raw = err.to_string();
    if raw.contains("AccessDenied") {
        "R2 refused the upload (Access Denied). The API token needs Object Read & Write on this bucket.".into()
    } else {
        format!("object store: {raw}")
    }
}

#[derive(Clone)]
pub struct R2 {
    client: Client,
    bucket: String,
}

#[derive(Clone, Debug)]
pub struct R2Config {
    pub account_id: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub bucket: String,
    pub endpoint: Option<String>,
}

impl R2Config {
    pub fn from_env() -> Result<Self, StorageError> {
        Ok(Self {
            account_id: env("R2_ACCOUNT_ID")?,
            access_key_id: env("R2_ACCESS_KEY_ID")?,
            secret_access_key: env("R2_SECRET_ACCESS_KEY")?,
            bucket: env("R2_BUCKET")?,
            endpoint: std::env::var("R2_ENDPOINT").ok().filter(|s| !s.is_empty()),
        })
    }
}

fn env(key: &'static str) -> Result<String, StorageError> {
    std::env::var(key)
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or(StorageError::MissingEnv(key))
}

impl R2 {
    pub async fn connect(cfg: R2Config) -> Result<Self, StorageError> {
        let endpoint = cfg
            .endpoint
            .unwrap_or_else(|| format!("https://{}.r2.cloudflarestorage.com", cfg.account_id));
        let creds = Credentials::new(cfg.access_key_id, cfg.secret_access_key, None, None, "r2");
        let shared = aws_config::defaults(BehaviorVersion::latest())
            .credentials_provider(creds)
            .region(Region::new("auto"))
            .endpoint_url(endpoint)
            .load()
            .await;
        let s3_cfg = aws_sdk_s3::config::Builder::from(&shared)
            .force_path_style(true)
            .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
            .response_checksum_validation(ResponseChecksumValidation::WhenRequired)
            .build();
        Ok(Self {
            client: Client::from_conf(s3_cfg),
            bucket: cfg.bucket,
        })
    }

    pub async fn from_env() -> Result<Self, StorageError> {
        Self::connect(R2Config::from_env()?).await
    }

    #[must_use]
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    pub async fn presign_put(
        &self,
        key: &str,
        content_type: &str,
        expires: Duration,
    ) -> Result<String, StorageError> {
        let presign = PresigningConfig::expires_in(expires)
            .map_err(|e| StorageError::Presign(e.to_string()))?;
        let req = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .presigned(presign)
            .await
            .map_err(|e| StorageError::Presign(e.to_string()))?;
        Ok(req.uri().to_string())
    }

    pub async fn presign_get(&self, key: &str, expires: Duration) -> Result<String, StorageError> {
        let presign = PresigningConfig::expires_in(expires)
            .map_err(|e| StorageError::Presign(e.to_string()))?;
        let req = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(presign)
            .await
            .map_err(|e| StorageError::Presign(e.to_string()))?;
        Ok(req.uri().to_string())
    }

    pub async fn get_bytes(&self, key: &str) -> Result<Vec<u8>, StorageError> {
        let out = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(aws_sdk_s3::Error::from)?;
        let data = out
            .body
            .collect()
            .await
            .map_err(|e| StorageError::Presign(e.to_string()))?
            .into_bytes();
        Ok(data.to_vec())
    }

    pub async fn put_bytes(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<(), StorageError> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .body(aws_sdk_s3::primitives::ByteStream::from(bytes))
            .send()
            .await
            .map_err(aws_sdk_s3::Error::from)?;
        Ok(())
    }

    pub async fn delete_object(&self, key: &str) -> Result<(), StorageError> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(aws_sdk_s3::Error::from)?;
        Ok(())
    }

    /// Delete every object whose key starts with `prefix`. Returns how many were removed.
    pub async fn delete_prefix(&self, prefix: &str) -> Result<u32, StorageError> {
        let mut token: Option<String> = None;
        let mut deleted = 0u32;
        loop {
            let mut req = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(prefix);
            if let Some(t) = &token {
                req = req.continuation_token(t);
            }
            let out = req.send().await.map_err(aws_sdk_s3::Error::from)?;
            let keys: Vec<String> = out
                .contents()
                .iter()
                .filter_map(|obj| obj.key().map(str::to_string))
                .collect();
            if !keys.is_empty() {
                deleted += self.delete_keys(&keys).await?;
            }
            if out.is_truncated() == Some(true) {
                token = out.next_continuation_token().map(str::to_string);
                if token.is_none() {
                    break;
                }
            } else {
                break;
            }
        }
        Ok(deleted)
    }

    async fn delete_keys(&self, keys: &[String]) -> Result<u32, StorageError> {
        use aws_sdk_s3::types::{Delete, ObjectIdentifier};
        let mut count = 0u32;
        for chunk in keys.chunks(1000) {
            let objects = chunk
                .iter()
                .map(|key| {
                    ObjectIdentifier::builder()
                        .key(key)
                        .build()
                        .map_err(|e| StorageError::Presign(e.to_string()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let delete = Delete::builder()
                .set_objects(Some(objects))
                .quiet(true)
                .build()
                .map_err(|e| StorageError::Presign(e.to_string()))?;
            self.client
                .delete_objects()
                .bucket(&self.bucket)
                .delete(delete)
                .send()
                .await
                .map_err(aws_sdk_s3::Error::from)?;
            count += chunk.len() as u32;
        }
        Ok(count)
    }
}
