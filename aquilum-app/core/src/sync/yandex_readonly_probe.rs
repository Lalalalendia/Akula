//! YC-S3-T01c (Core slice): deliberately bounded, user-invoked Yandex S3
//! read-only authentication probe. This module has no File Gate, remote Vault
//! authority, background poller, writer, delete operation or Sync activation.
//!
//! The caller must obtain credentials from an approved OS-protected provider.
//! It must never store keys in AppConfig, SQLite, notes, URLs or logs.
//! The AWS SDK internally retains credential copies for the short-lived request;
//! zeroizing caller buffers does not prove that SDK/allocator copies are erased.

use crate::settings::models::YandexS3Onboarding;
use aws_sdk_s3::config::{Credentials, Region, retry::RetryConfig};
use aws_sdk_s3::error::{ProvideErrorMetadata, SdkError};
use std::time::Duration;

pub const YANDEX_S3_ENDPOINT: &str = "https://storage.yandexcloud.net";
pub const YANDEX_S3_REGION: &str = "ru-central1";
// Only one object can be returned; no object names or response bodies are
// retained. This does not inventory a bucket or identify a remote Akula Vault.
const PROBE_PREFIX: &str = "akula-sync/";
const PROBE_MAX_KEYS: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YandexS3ProbeError {
    InvalidBucket,
    MissingKeys,
    InvalidKeyFormat,
    CredentialsRejected,
    AccessDenied,
    BucketNotFound,
    ClockSkew,
    WrongRegion,
    Timeout,
    NetworkOrTls,
    ServiceUnavailable,
}

/// A successful bounded LIST establishes only that this specific credential
/// pair can list the selected bucket/prefix at the time of this request.
/// Never convert Ok(()) to remote ownership, Sync permission, writable storage,
/// S3 CAS support, local preservation or a durable trusted Vault baseline.
pub async fn probe_yandex_s3_read_only(
    bucket: &str,
    access_key_id: &str,
    secret_access_key: &str,
) -> Result<(), YandexS3ProbeError> {
    validate_input(bucket, access_key_id, secret_access_key)?;

    // Do not allow AWS_PROFILE, shared credential files or environment keys
    // to silently replace the Windows Credential Manager key pair.
    // Explicit region and endpoint override shared/environment endpoints.
    let credentials = Credentials::new(
        access_key_id.to_owned(),
        secret_access_key.to_owned(),
        None,
        None,
        "akula-yandex-readonly-probe",
    );
    let shared = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(Region::new(YANDEX_S3_REGION))
        .credentials_provider(credentials)
        .timeout_config(
            aws_config::timeout::TimeoutConfig::builder()
                .connect_timeout(Duration::from_secs(4))
                .operation_timeout(Duration::from_secs(12))
                .build(),
        )
        .load()
        .await;
    let config = aws_sdk_s3::config::Builder::from(&shared)
        .endpoint_url(YANDEX_S3_ENDPOINT)
        .force_path_style(true)
        // One explicit read-only request; never retry incorrect or revoked
        // credentials and never run an automatic background LIST loop.
        .retry_config(RetryConfig::standard().with_max_attempts(1))
        .build();

    let client = aws_sdk_s3::Client::from_conf(config);
    probe_with_client(&client, bucket).await
}

// This small request boundary is separately exercised against disposable
// localhost MinIO. The production caller above constructs a *fixed* Yandex
// endpoint; no public method accepts an arbitrary probe destination.
async fn probe_with_client(
    client: &aws_sdk_s3::Client,
    bucket: &str,
) -> Result<(), YandexS3ProbeError> {
    client
        .list_objects_v2()
        .bucket(bucket)
        .prefix(PROBE_PREFIX)
        .max_keys(PROBE_MAX_KEYS)
        .send()
        .await
        .map_err(|error| {
            if matches!(&error, SdkError::TimeoutError(_)) {
                return YandexS3ProbeError::Timeout;
            }
            if matches!(&error, SdkError::DispatchFailure(_)) {
                return YandexS3ProbeError::NetworkOrTls;
            }
            classify_service_error(error.as_service_error().and_then(|service| service.code()))
        })?;

    Ok(())
}

fn validate_input(
    bucket: &str,
    access_key_id: &str,
    secret_access_key: &str,
) -> Result<(), YandexS3ProbeError> {
    // The onboarding *draft* allows an empty bucket; network admission must not.
    if bucket.is_empty() || !YandexS3Onboarding::valid_bucket(bucket) {
        return Err(YandexS3ProbeError::InvalidBucket);
    }
    if access_key_id.is_empty() || secret_access_key.is_empty() {
        return Err(YandexS3ProbeError::MissingKeys);
    }
    // Matches the native WinCred acceptance limits. In particular, prohibit
    // control characters and any key-like data entering a URL or log label.
    if !(3..=128).contains(&access_key_id.len())
        || !(8..=255).contains(&secret_access_key.len())
        || !access_key_id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        || !secret_access_key.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(YandexS3ProbeError::InvalidKeyFormat);
    }
    Ok(())
}

/// Classify only the machine-readable service code. Do not format SDK errors
/// or include authorization headers, key IDs, secret bytes or server bodies
/// in any user-visible notice or logs.
fn classify_service_error(code: Option<&str>) -> YandexS3ProbeError {
    match code {
        Some("InvalidAccessKeyId" | "SignatureDoesNotMatch" | "InvalidSecurity"
            | "InvalidToken" | "ExpiredToken") => YandexS3ProbeError::CredentialsRejected,
        Some("AccessDenied" | "AllAccessDisabled" | "InvalidPayer" | "NotSignedUp") =>
            YandexS3ProbeError::AccessDenied,
        Some("NoSuchBucket") => YandexS3ProbeError::BucketNotFound,
        Some("RequestTimeTooSkewed") => YandexS3ProbeError::ClockSkew,
        Some("AuthorizationHeaderMalformed" | "PermanentRedirect" | "IncorrectEndpoint") =>
            YandexS3ProbeError::WrongRegion,
        // A 403/404 without an interpretable XML code cannot establish which
        // bucket or credential was wrong. Fail closed, never guess success.
        _ => YandexS3ProbeError::ServiceUnavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_bucket_or_secrets_are_accepted_by_admission() {
        assert_eq!(validate_input("", "valid_key", "test-secret-123"),
            Err(YandexS3ProbeError::InvalidBucket));
        assert_eq!(validate_input("bad.name", "valid_key", "test-secret-123"),
            Err(YandexS3ProbeError::InvalidBucket));
        assert_eq!(validate_input("akula-private-bucket", "", "test-secret-123"),
            Err(YandexS3ProbeError::MissingKeys));
        assert_eq!(validate_input("akula-private-bucket", "valid_key", ""),
            Err(YandexS3ProbeError::MissingKeys));
        assert_eq!(validate_input("akula-private-bucket", "valid_key", "secret\nhidden"),
            Err(YandexS3ProbeError::InvalidKeyFormat));
        assert_eq!(validate_input("akula-private-bucket", "valid_key", "test-secret-123"),
            Ok(()));
    }

    #[test]
    fn machine_codes_do_not_conflate_permissions_credentials_and_missing_bucket() {
        for code in ["InvalidAccessKeyId", "SignatureDoesNotMatch", "InvalidSecurity", "ExpiredToken"] {
            assert_eq!(classify_service_error(Some(code)), YandexS3ProbeError::CredentialsRejected);
        }
        for code in ["AccessDenied", "InvalidPayer", "NotSignedUp"] {
            assert_eq!(classify_service_error(Some(code)), YandexS3ProbeError::AccessDenied);
        }
        assert_eq!(classify_service_error(Some("NoSuchBucket")), YandexS3ProbeError::BucketNotFound);
        assert_eq!(classify_service_error(Some("RequestTimeTooSkewed")), YandexS3ProbeError::ClockSkew);
        assert_eq!(classify_service_error(Some("AuthorizationHeaderMalformed")), YandexS3ProbeError::WrongRegion);
        assert_eq!(classify_service_error(None), YandexS3ProbeError::ServiceUnavailable);
        assert_eq!(classify_service_error(Some("Arbitrary")), YandexS3ProbeError::ServiceUnavailable);
    }


    /// Exercises the exact production LIST builder against a disposable
    /// localhost S3 server. Fixture management (create/delete bucket) is
    /// test-only; probe_with_client performs no writes or file operations.
    #[tokio::test]
    #[ignore = "requires CI's isolated localhost MinIO"]
    async fn live_minio_yandex_probe_request_is_read_only() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT").expect("local S3 test endpoint");
        // Fail closed if the test was pointed at a real S3 account.
        assert_eq!(endpoint, "http://127.0.0.1:9000");
        let access = std::env::var("AWS_ACCESS_KEY_ID").expect("CI synthetic access key");
        let secret = std::env::var("AWS_SECRET_ACCESS_KEY").expect("CI synthetic secret");

        async fn client(endpoint: &str, access: &str, secret: &str) -> aws_sdk_s3::Client {
            let creds = Credentials::new(access.to_owned(), secret.to_owned(), None, None, "minio-only");
            let shared = aws_config::defaults(aws_config::BehaviorVersion::latest())
                .region(Region::new("us-east-1"))
                .credentials_provider(creds)
                .load()
                .await;
            let config = aws_sdk_s3::config::Builder::from(&shared)
                .endpoint_url(endpoint)
                .force_path_style(true)
                .retry_config(RetryConfig::standard().with_max_attempts(1))
                .build();
            aws_sdk_s3::Client::from_conf(config)
        }

        let admin = client(&endpoint, &access, &secret).await;
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
        let bucket = format!("akula-yc-probe-{}-{nonce}", std::process::id());
        assert!(YandexS3Onboarding::valid_bucket(&bucket));
        assert!(admin.create_bucket().bucket(&bucket).send().await.is_ok(),
            "could not create disposable MinIO-only fixture");
        // Empty bucket is healthy; absence of a matching object is not failure.
        assert_eq!(probe_with_client(&admin, &bucket).await, Ok(()));
        assert_eq!(probe_with_client(&admin, &format!("{bucket}-missing")).await,
            Err(YandexS3ProbeError::BucketNotFound));

        let untrusted = client(&endpoint, "akula-invalid-probe-key", "invalid-probe-secret").await;
        assert_eq!(probe_with_client(&untrusted, &bucket).await,
            Err(YandexS3ProbeError::CredentialsRejected));
        assert!(admin.delete_bucket().bucket(&bucket).send().await.is_ok(),
            "could not delete disposable MinIO-only fixture");
    }

    #[test]
    fn endpoint_prefix_and_request_cap_are_fixed_not_from_user_input() {
        assert_eq!(YANDEX_S3_ENDPOINT, "https://storage.yandexcloud.net");
        assert_eq!(YANDEX_S3_REGION, "ru-central1");
        assert_eq!(PROBE_PREFIX, "akula-sync/");
        assert_eq!(PROBE_MAX_KEYS, 1);
    }
}
