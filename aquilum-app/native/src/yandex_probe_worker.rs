//! Explicit, feature-gated Windows read-only S3 authorization test.
//! Runs ONLY from a user-requested background worker. It cannot list local
//! files, create remote objects, mutate journals or start synchronization.
//!
//! The short-lived callback owns only borrowed, bucket-bound WinCred strings.
//! AWS SDK may internally copy them; OS credential deletion does not revoke
//! the corresponding service-account key in Yandex Cloud.

use crate::yandex_credentials::{self, VaultError};
use crate::yandex_probe_session::{ProbeFailure, ProbeOutcome};
use aquilum_core::sync::YandexS3ProbeError;

pub fn check(bucket: &str) -> ProbeOutcome {
    let result = yandex_credentials::with_stored_keys(bucket, |id, secret| {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(_) => return ProbeOutcome::Failed(ProbeFailure::ServiceUnavailable),
        };
        // No SDK error, request metadata, credentials or object keys are logged.
        // This is the sole network entry point and remains disabled in
        // normal Windows builds without yandex-s3-connection-preview.
        match runtime.block_on(
            aquilum_core::sync::probe_yandex_s3_read_only(bucket, id, secret)
        ) {
            Ok(()) => {
                // The user might have rotated keys outside Akula's Settings
                // while the request was in flight. Re-read the OS credential
                // at the end; never keep a success for a different key pair.
                // This remains a point-in-time observation, not permanent
                // authorization, and does not grant remote Sync authority.
                let unchanged = yandex_credentials::with_stored_keys(
                    bucket,
                    |updated_id, updated_secret| id == updated_id && secret == updated_secret,
                ).unwrap_or(false);
                if unchanged {
                    ProbeOutcome::ReadOnlyListVerified
                } else {
                    ProbeOutcome::Failed(ProbeFailure::KeysChangedDuringCheck)
                }
            },
            Err(code) => ProbeOutcome::Failed(classify(code)),
        }
    });
    match result {
        Ok(value) => value,
        Err(VaultError::MissingKeys) => ProbeOutcome::Failed(ProbeFailure::MissingKeys),
        Err(VaultError::InvalidKeys | VaultError::InvalidBucket) =>
            ProbeOutcome::Failed(ProbeFailure::InvalidStoredKeys),
        Err(VaultError::Unavailable | VaultError::Unsupported) =>
            ProbeOutcome::Failed(ProbeFailure::CredentialStoreUnavailable),
    }
}

fn classify(code: YandexS3ProbeError) -> ProbeFailure {
    match code {
        YandexS3ProbeError::InvalidBucket | YandexS3ProbeError::InvalidKeyFormat =>
            ProbeFailure::InvalidStoredKeys,
        YandexS3ProbeError::MissingKeys => ProbeFailure::MissingKeys,
        YandexS3ProbeError::CredentialsRejected => ProbeFailure::CredentialsRejected,
        YandexS3ProbeError::AccessDenied => ProbeFailure::AccessDenied,
        YandexS3ProbeError::BucketNotFound => ProbeFailure::BucketNotFound,
        YandexS3ProbeError::ClockSkew => ProbeFailure::ClockSkew,
        YandexS3ProbeError::WrongRegion => ProbeFailure::WrongRegion,
        YandexS3ProbeError::Timeout => ProbeFailure::Timeout,
        YandexS3ProbeError::NetworkOrTls => ProbeFailure::NetworkOrTls,
        YandexS3ProbeError::ServiceUnavailable => ProbeFailure::ServiceUnavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fail_closed_translation_never_promises_remote_write_or_sync() {
        assert_eq!(classify(YandexS3ProbeError::CredentialsRejected),
            ProbeFailure::CredentialsRejected);
        assert_eq!(classify(YandexS3ProbeError::AccessDenied),
            ProbeFailure::AccessDenied);
        assert_eq!(classify(YandexS3ProbeError::BucketNotFound),
            ProbeFailure::BucketNotFound);
        assert_eq!(classify(YandexS3ProbeError::Timeout),
            ProbeFailure::Timeout);
        assert_eq!(classify(YandexS3ProbeError::MissingKeys),
            ProbeFailure::MissingKeys);
    }
}
