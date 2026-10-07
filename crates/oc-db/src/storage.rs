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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectStat {
    pub bytes: u64,
    pub modified_ms: u64,
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

fn object_missing(
    err: &aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::head_object::HeadObjectError>,
) -> bool {
    use aws_sdk_s3::error::SdkError;
    use aws_sdk_s3::operation::head_object::HeadObjectError;

    use aws_sdk_s3::error::ProvideErrorMetadata;

    let SdkError::ServiceError(service) = err else {
        return false;
    };
    if matches!(service.err(), HeadObjectError::NotFound(_)) {
        return true;
    }
    service.raw().status().as_u16() == 404 || service.err().code() == Some("NoSuchKey")
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

    /// `Ok(None)` when the key is not in the bucket.
    pub async fn stat_object(&self, key: &str) -> Result<Option<ObjectStat>, StorageError> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(out) => {
                let modified_ms = out
                    .last_modified()
                    .and_then(|stamp| stamp.to_millis().ok())
                    .filter(|ms| *ms >= 0)
                    .unwrap_or(0) as u64;
                let bytes = out.content_length().unwrap_or(0).max(0) as u64;
                Ok(Some(ObjectStat { bytes, modified_ms }))
            }
            Err(err) if object_missing(&err) => Ok(None),
            Err(err) => Err(StorageError::from(aws_sdk_s3::Error::from(err))),
        }
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
        // One connection under-fills a link to Cloudflare. Larger files go up
        // as parallel parts. R2 requires every part but the last to be at least 5 MiB.
        const PART: usize = 8 * 1024 * 1024;
        if bytes.len() <= PART {
            self.client
                .put_object()
                .bucket(&self.bucket)
                .key(key)
                .content_type(content_type)
                .body(aws_sdk_s3::primitives::ByteStream::from(bytes))
                .send()
                .await
                .map_err(aws_sdk_s3::Error::from)?;
            return Ok(());
        }
        self.put_parts(key, bytes, content_type, PART).await
    }

    async fn put_parts(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
        part_size: usize,
    ) -> Result<(), StorageError> {
        use aws_sdk_s3::types::CompletedMultipartUpload;

        let chunks = split_parts(bytes, part_size);
        let created = self
            .client
            .create_multipart_upload()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .send()
            .await
            .map_err(aws_sdk_s3::Error::from)?;
        let Some(upload_id) = created.upload_id().map(str::to_string) else {
            return Err(StorageError::Sdk("R2 did not return an upload id".into()));
        };
        tracing::info!(key, parts = chunks.len(), "multipart upload");
        let uploaded = self.send_parts(key, &upload_id, chunks).await;
        let parts = match uploaded {
            Ok(parts) => parts,
            Err(err) => {
                let _ = self
                    .client
                    .abort_multipart_upload()
                    .bucket(&self.bucket)
                    .key(key)
                    .upload_id(&upload_id)
                    .send()
                    .await;
                return Err(err);
            }
        };
        let completed = CompletedMultipartUpload::builder()
            .set_parts(Some(parts))
            .build();
        if let Err(err) = self
            .client
            .complete_multipart_upload()
            .bucket(&self.bucket)
            .key(key)
            .upload_id(&upload_id)
            .multipart_upload(completed)
            .send()
            .await
        {
            let _ = self
                .client
                .abort_multipart_upload()
                .bucket(&self.bucket)
                .key(key)
                .upload_id(&upload_id)
                .send()
                .await;
            return Err(aws_sdk_s3::Error::from(err).into());
        }
        Ok(())
    }

    async fn send_parts(
        &self,
        key: &str,
        upload_id: &str,
        chunks: Vec<Vec<u8>>,
    ) -> Result<Vec<aws_sdk_s3::types::CompletedPart>, StorageError> {
        use aws_sdk_s3::types::CompletedPart;

        const IN_FLIGHT: usize = 4;
        let mut pending = chunks.into_iter().enumerate();
        let mut set = tokio::task::JoinSet::new();
        let mut inflight = 0usize;
        let mut done = Vec::new();
        loop {
            while inflight < IN_FLIGHT {
                let Some((index, chunk)) = pending.next() else {
                    break;
                };
                let part_number = (index + 1) as i32;
                let client = self.client.clone();
                let bucket = self.bucket.clone();
                let key = key.to_string();
                let upload_id = upload_id.to_string();
                set.spawn(async move {
                    let out = client
                        .upload_part()
                        .bucket(bucket)
                        .key(key)
                        .upload_id(upload_id)
                        .part_number(part_number)
                        .body(aws_sdk_s3::primitives::ByteStream::from(chunk))
                        .send()
                        .await
                        .map_err(aws_sdk_s3::Error::from)?;
                    let etag = out
                        .e_tag()
                        .ok_or_else(|| StorageError::Sdk("R2 part returned no etag".into()))?;
                    Ok::<_, StorageError>(
                        CompletedPart::builder()
                            .part_number(part_number)
                            .e_tag(etag)
                            .build(),
                    )
                });
                inflight += 1;
            }
            let Some(joined) = set.join_next().await else {
                break;
            };
            inflight -= 1;
            done.push(joined.map_err(|err| StorageError::Sdk(err.to_string()))??);
        }
        done.sort_by_key(|part| part.part_number().unwrap_or(0));
        Ok(done)
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

/// Split `bytes` into owned pieces of `part_size`. The last piece is the remainder.
fn split_parts(mut bytes: Vec<u8>, part_size: usize) -> Vec<Vec<u8>> {
    let mut parts = Vec::new();
    while bytes.len() > part_size {
        let rest = bytes.split_off(part_size);
        parts.push(bytes);
        bytes = rest;
    }
    if !bytes.is_empty() {
        parts.push(bytes);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::split_parts;

    #[test]
    fn a_large_upload_splits_into_full_parts() {
        let part = 8;
        let bytes = vec![1u8; 20];
        let parts = split_parts(bytes, part);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 8);
        assert_eq!(parts[2].len(), 4);
        assert!(parts[..parts.len() - 1].iter().all(|part| part.len() >= 5));
    }

    #[test]
    fn a_short_upload_stays_one_part() {
        let parts = split_parts(vec![7u8; 8], 8);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].len(), 8);
    }
}
