//! Explicit, scoped S3 key provider for the installed Windows application.
//!
//! Never use ambient AWS_PROFILE, environment keys, metadata credentials or
//! profile files for a user-imported S3 pairing. SDK clients may internally
//! copy signed credentials; do not return a client to a GUI render callback.

use super::{
    AwsS3ProbeClient, S3ConnectionProfile, S3Keyspace, S3PutCondition,
    S3TransportError,
};
use crate::sync::OperationId;
use aws_sdk_s3::config::Credentials;
#[cfg(test)]
use aws_sdk_s3::error::ProvideErrorMetadata;
use std::time::Duration;
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use super::S3TestIoCounters;

/// A bounded, network-verified access observation, not a trust root, File
/// Gate writer, durable remote receipt, or capability to replace any note.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum S3PairAccess {
    ReadOnly,
    ReadWrite,
}

impl AwsS3ProbeClient {
    /// Explicit credentials sourced from one selected native OS keychain item.
    /// SDK config may still resolve non-secret environment timeouts/region,
    /// but the signed S3 identity cannot fall back to ambient AWS credentials.
    pub async fn connect_with_explicit_credentials(
        profile: S3ConnectionProfile,
        keyspace: S3Keyspace,
        access_key_id: &str,
        secret_access_key: &str,
    ) -> Result<Self, S3TransportError> {
        profile.validate()?;
        if !(3..=128).contains(&access_key_id.len())
            || !access_key_id.bytes().all(|b| b.is_ascii_alphanumeric()
                || b == b'_' || b == b'-')
            || !(8..=255).contains(&secret_access_key.len())
            || !secret_access_key.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(S3TransportError::InvalidProfile);
        }
        let fixed = Credentials::new(
            access_key_id,
            secret_access_key,
            None,
            None,
            "akula-native-selected-wincred",
        );
        let shared = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(profile.region.clone()))
            .credentials_provider(fixed.clone())
            .timeout_config(
                aws_config::timeout::TimeoutConfig::builder()
                    .connect_timeout(Duration::from_secs(5))
                    .operation_timeout(Duration::from_secs(20))
                    .build(),
            )
            .load()
            .await;
        let mut config = aws_sdk_s3::config::Builder::from(&shared)
            .credentials_provider(fixed.clone())
            .force_path_style(profile.force_path_style);
        if let Some(endpoint) = profile.endpoint.clone() {
            config = config.endpoint_url(endpoint);
        } else {
            config.set_endpoint_url(None);
        }
        let sts_client = if profile.endpoint.is_none() {
            let mut sts = aws_sdk_sts::config::Builder::from(&shared)
                .credentials_provider(fixed);
            sts.set_endpoint_url(None);
            Some(aws_sdk_sts::Client::from_conf(sts.build()))
        } else {
            None
        };
        Ok(Self {
            client: aws_sdk_s3::Client::from_conf(config.build()),
            bucket: profile.bucket.clone(),
            keyspace,
            profile,
            sts_client,
            #[cfg(test)]
            test_io: Arc::new(S3TestIoCounters::default()),
        })
    }

    /// A successful LIST is only read access. Try an immutable scoped object
    /// once; read it back and compare bytes + BLAKE3 before reporting R/W.
    /// The tiny content-addressed proof object remains unreferenced forever:
    /// no deletion and NO mutation to refs/current.json is performed.
    pub async fn probe_pair_read_write(&self) -> Result<S3PairAccess, S3TransportError> {
        self.diagnostic_list_page(None, 1).await?;
        let payload = format!("akula-connectivity-v1:{}", OperationId::new());
        let expected = blake3::hash(payload.as_bytes()).to_hex().to_string();
        let key = self.keyspace.immutable_blob_key(&expected)
            .map_err(|_| S3TransportError::OutsideVaultNamespace)?;
        match self.put_conditionally(
            &key, payload.as_bytes(), S3PutCondition::IfAbsent,
        ).await {
            Ok(_) | Err(S3TransportError::CommitUncertain)
            | Err(S3TransportError::PreconditionFailed) => {}
            Err(S3TransportError::PermissionDenied) => return Ok(S3PairAccess::ReadOnly),
            Err(other) => return Err(other),
        }
        let back = self.get_bounded(&key, payload.len() as u64).await?;
        if back.bytes != payload.as_bytes() || back.blake3 != expected {
            return Err(S3TransportError::ChangedDuringRead);
        }
        Ok(S3PairAccess::ReadWrite)
    }
}

#[cfg(test)]
impl AwsS3ProbeClient {
    #[cfg(test)]
    pub(crate) async fn test_ensure_minio_bucket(&self) -> Result<(), S3TransportError> {
        match self.client.create_bucket().bucket(&self.bucket).send().await {
            Ok(_) => Ok(()),
            Err(error) if error.as_service_error()
                .and_then(|service| service.code()) == Some("BucketAlreadyOwnedByYou") => Ok(()),
            Err(_) => Err(S3TransportError::Unavailable),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalid_explicit_credentials_fail_before_network() {
        let profile = S3ConnectionProfile {
            bucket: "akula-test".into(),
            region: "ru-central1".into(),
            endpoint: Some("https://storage.yandexcloud.net".into()),
            force_path_style: true,
        };
        let ns = S3Keyspace::new("akula-sync/alpha", crate::sync::VaultId::new()).unwrap();
        let res = AwsS3ProbeClient::connect_with_explicit_credentials(
            profile, ns, "invalid id!", "not-a-real-secret",
        ).await;
        assert!(matches!(res, Err(S3TransportError::InvalidProfile)));
    }

    #[tokio::test]
    #[ignore = "requires disposable MinIO integration server"]
    async fn probe_private_minio_requires_explicit_credential_and_readback() {
        let Ok(endpoint) = std::env::var("AQ_S3_TEST_ENDPOINT") else { return };
        assert!(endpoint.starts_with("http://localhost:")
            || endpoint.starts_with("http://127.0.0.1:"));
        let profile = S3ConnectionProfile {
            bucket: "akula-pair-probe".into(),
            region: "us-east-1".into(),
            endpoint: Some(endpoint),
            force_path_style: true,
        };
        let ns = S3Keyspace::new("akula-sync/probe", crate::sync::VaultId::new()).unwrap();
        // The test harness supplies disposable MinIO credentials. A sealed
        // real native client never falls back to environment credentials.
        let id = std::env::var("AWS_ACCESS_KEY_ID").expect("MinIO fixture key ID");
        let secret = std::env::var("AWS_SECRET_ACCESS_KEY").expect("MinIO fixture secret");
        let client = AwsS3ProbeClient::connect_with_explicit_credentials(
            profile.clone(), ns, &id, &secret,
        ).await.unwrap();
        match client.client.create_bucket().bucket(&profile.bucket).send().await {
            Ok(_) => {}
            Err(error) if error.as_service_error()
                .and_then(|service| service.code())
                == Some("BucketAlreadyOwnedByYou") => {}
            Err(error) => panic!("private MinIO bucket provisioning: {error}"),
        }
        assert_eq!(client.probe_pair_read_write().await.unwrap(), S3PairAccess::ReadWrite);
    }
}
