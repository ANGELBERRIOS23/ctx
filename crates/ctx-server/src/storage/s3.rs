//! AWS S3 and MinIO object storage implementation of [`StorageBackend`].
//!
//! [`S3BlobStore`] manages binary blob and synchronization snapshot storage
//! within an S3-compatible bucket using the AWS SDK for Rust ([`aws_sdk_s3`]).

use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;

use super::{Result, StorageBackend, StorageError};

/// S3/MinIO-compatible object storage backend for binary blobs and project snapshots.
#[derive(Debug, Clone)]
pub struct S3BlobStore {
    client: Client,
    bucket: String,
    prefix: Option<String>,
}

impl S3BlobStore {
    /// Creates a new [`S3BlobStore`] for the specified bucket without key prefixing.
    pub fn new(client: Client, bucket: impl Into<String>) -> Self {
        Self {
            client,
            bucket: bucket.into(),
            prefix: None,
        }
    }

    /// Creates a new [`S3BlobStore`] with an optional namespace prefix prepended to all keys.
    pub fn with_prefix(
        client: Client,
        bucket: impl Into<String>,
        prefix: impl Into<String>,
    ) -> Self {
        let prefix_str = prefix.into();
        let trimmed = prefix_str.trim().trim_matches('/');
        let prefix = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };

        Self {
            client,
            bucket: bucket.into(),
            prefix,
        }
    }

    /// Returns a reference to the configured S3 bucket name.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// Returns the optional key prefix, if configured.
    pub fn prefix(&self) -> Option<&str> {
        self.prefix.as_deref()
    }

    /// Returns a reference to the underlying [`Client`].
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Resolves a key into its full S3 object key, applying the configured prefix if present.
    pub fn resolve_key(&self, key: &str) -> String {
        let trimmed = key.trim().trim_start_matches('/');
        match &self.prefix {
            Some(prefix) => format!("{prefix}/{trimmed}"),
            None => trimmed.to_string(),
        }
    }

    /// Helper to construct an S3 client configured for standard AWS or custom S3-compatible endpoints like MinIO.
    pub async fn build_client(
        endpoint_url: Option<&str>,
        region: Option<&str>,
        force_path_style: bool,
    ) -> Client {
        let mut config_loader = aws_config::defaults(aws_config::BehaviorVersion::latest());
        if let Some(reg) = region {
            config_loader = config_loader.region(aws_sdk_s3::config::Region::new(reg.to_string()));
        }
        let sdk_config = config_loader.load().await;

        let mut s3_config_builder = aws_sdk_s3::config::Builder::from(&sdk_config);
        if let Some(endpoint) = endpoint_url {
            s3_config_builder = s3_config_builder.endpoint_url(endpoint);
        }
        if force_path_style {
            s3_config_builder = s3_config_builder.force_path_style(true);
        }

        Client::from_conf(s3_config_builder.build())
    }
}

impl StorageBackend for S3BlobStore {
    async fn save_blob(&self, key: &str, data: &[u8]) -> Result<()> {
        let full_key = self.resolve_key(key);
        let body = ByteStream::from(data.to_vec());

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .body(body)
            .send()
            .await
            .map_err(|err| StorageError::S3(err.to_string()))?;

        Ok(())
    }

    async fn get_blob(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let full_key = self.resolve_key(key);

        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .send()
            .await;

        match resp {
            Ok(output) => {
                let bytes = output
                    .body
                    .collect()
                    .await
                    .map_err(|err| StorageError::S3(err.to_string()))?
                    .into_bytes()
                    .to_vec();
                Ok(Some(bytes))
            }
            Err(err) => {
                if let SdkError::ServiceError(ref service_err) = err {
                    if service_err.err().is_no_such_key() {
                        return Ok(None);
                    }
                }
                Err(StorageError::S3(err.to_string()))
            }
        }
    }

    async fn delete_blob(&self, key: &str) -> Result<()> {
        let full_key = self.resolve_key(key);

        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .send()
            .await
            .map_err(|err| StorageError::S3(err.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_s3_blob_store_key_resolution() {
        let conf = aws_sdk_s3::Config::builder()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .region(aws_sdk_s3::config::Region::new("us-east-1"))
            .build();
        let client = Client::from_conf(conf);

        let store_no_prefix = S3BlobStore::new(client.clone(), "my-bucket");
        assert_eq!(store_no_prefix.bucket(), "my-bucket");
        assert_eq!(store_no_prefix.prefix(), None);
        assert_eq!(
            store_no_prefix.resolve_key("snapshots/proj/1.json"),
            "snapshots/proj/1.json"
        );
        assert_eq!(
            store_no_prefix.resolve_key("/leading/slash.bin"),
            "leading/slash.bin"
        );

        let store_with_prefix =
            S3BlobStore::with_prefix(client, "my-bucket", "environments/prod/");
        assert_eq!(store_with_prefix.prefix(), Some("environments/prod"));
        assert_eq!(
            store_with_prefix.resolve_key("snapshots/proj/1.json"),
            "environments/prod/snapshots/proj/1.json"
        );
    }

    #[test]
    fn test_s3_blob_store_implements_storage_backend() {
        fn assert_storage_backend<T: StorageBackend>() {}
        assert_storage_backend::<S3BlobStore>();
    }
}
