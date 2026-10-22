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
    check_with_operation(bucket, |selected_bucket, id, secret| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| YandexS3ProbeError::ServiceUnavailable)?;
        // This is the only production network operation. Endpoint, region and
        // LIST limits are fixed by Core; never log SDK error data or keys.
        runtime.block_on(aquilum_core::sync::probe_yandex_s3_read_only(
            selected_bucket, id, secret,
        ))
    })
}

/// Keep the real WinCred -> transport -> WinCred recheck boundary intact.
/// Test injection replaces only the transport, **not** the OS credential
/// store or the background ProbeSession. The callback is synchronous in
/// the worker thread and cannot return credentials to GUI state.
fn check_with_operation(
    bucket: &str,
    operation: impl FnOnce(&str, &str, &str) -> Result<(), YandexS3ProbeError>,
) -> ProbeOutcome {
    let result = yandex_credentials::with_stored_keys(bucket, |id, secret| {
        match operation(bucket, id, secret) {
            Ok(()) => {
                // The user might have rotated keys outside Akula's Settings
                // while the request was in flight. Re-read the OS credential
                // at the end; never keep a success for a different key pair.
                // This remains a point-in-time observation, not permanent
                // authorization, and does not grant remote Sync authority.
                let unchanged = yandex_credentials::still_matches(bucket, id, secret);
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


    /// Full synthetic WinCred -> injected LIST result -> single-flight session
    /// round-trip on a real Windows credential store, without network, files or
    /// user input. This closes the gap between independent Core/UI smoke tests.
    #[cfg(target_os = "windows")]
    #[test]
    fn protected_credential_rotation_during_network_reply_is_not_a_false_success() {
        use crate::yandex_probe_session::{ProbeSession, ProbeView};
        use std::sync::{mpsc, Arc};
        use std::time::{Duration, SystemTime, UNIX_EPOCH};
        use windows::core::PWSTR;
        use windows::Win32::Security::Credentials::{
            CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
        };
        use zeroize::Zeroizing;

        let suffix = SystemTime::now().duration_since(UNIX_EPOCH)
            .expect("clock").as_nanos();
        let bucket = format!("akula-probe-{}-{suffix}", std::process::id());

        struct DisposableCredential(String);
        impl Drop for DisposableCredential {
            fn drop(&mut self) {
                let _ = yandex_credentials::remove(&self.0);
            }
        }
        let _cleanup = DisposableCredential(bucket.clone());

        fn write_disposable_key(bucket: &str, secret: &str) {
            let target = yandex_credentials::target_name(bucket).expect("synthetic bucket");
            let mut target_w: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
            let mut user_w: Vec<u16> = "akula-bridge-test-id".encode_utf16()
                .chain(Some(0)).collect();
            let mut secret_w = Zeroizing::new(secret.as_bytes().to_vec());
            let credential = CREDENTIALW {
                Type: CRED_TYPE_GENERIC,
                TargetName: PWSTR(target_w.as_mut_ptr()),
                UserName: PWSTR(user_w.as_mut_ptr()),
                CredentialBlobSize: secret_w.len() as u32,
                CredentialBlob: secret_w.as_mut_ptr(),
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                ..Default::default()
            };
            unsafe { CredWriteW(&credential, 0) }
                .expect("isolated synthetic Windows credential write");
        }

        const OLD: &str = "only-synthetic-bridge-secret-A!";
        const NEW: &str = "only-synthetic-bridge-secret-B!";
        write_disposable_key(&bucket, OLD);
        let (wake_tx, wake_rx) = mpsc::channel::<()>();
        let mut session = ProbeSession::new(Arc::new(move || {
            let _ = wake_tx.send(());
        }));
        session.set_bucket(&bucket);

        // Positive path proves that the real protected credential can reach
        // the worker, but that no actual remote endpoint was contacted.
        let first_bucket = bucket.clone();
        session.start(true, move || check_with_operation(&first_bucket, |_, id, secret| {
            assert!(id == "akula-bridge-test-id" && secret == OLD);
            Ok(())
        })).expect("start synthetic successful worker");
        wake_rx.recv_timeout(Duration::from_secs(10)).expect("first worker");
        assert!(session.poll());
        assert_eq!(session.view_for(&bucket), ProbeView::ReadOnlyListVerified);

        // Start a second request. The mocked network cannot finish until
        // its caller allows it, so the original WinCred key is guaranteed to
        // have been checked out before an external overwrite of that target.
        let (entered_tx, entered_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let second_bucket = bucket.clone();
        session.start(true, move || check_with_operation(&second_bucket, |_, id, secret| {
            assert!(id == "akula-bridge-test-id" && secret == OLD);
            entered_tx.send(()).expect("signal signed-list phase");
            release_rx.recv_timeout(Duration::from_secs(10))
                .expect("wait for simulated network response");
            Ok(())
        })).expect("start second synthetic worker");
        entered_rx.recv_timeout(Duration::from_secs(10))
            .expect("original key was not checked out");
        write_disposable_key(&bucket, NEW);
        release_tx.send(()).expect("release simulated LIST");
        wake_rx.recv_timeout(Duration::from_secs(10)).expect("second worker");
        assert!(session.poll());
        assert_eq!(
            session.view_for(&bucket),
            ProbeView::Failed(ProbeFailure::KeysChangedDuringCheck),
            "successful remote LIST must not verify a locally rotated credential"
        );

        // Deleting the target after an earlier success may not leave a
        // lingering cached success, even if the GUI passed a stale "present".
        yandex_credentials::remove(&bucket).expect("delete synthetic key");
        let third_bucket = bucket.clone();
        session.start(true, move || check_with_operation(&third_bucket, |_, _, _| {
            panic!("transport must not execute when the WinCred target is missing");
        })).expect("start third synthetic worker");
        wake_rx.recv_timeout(Duration::from_secs(10)).expect("third worker");
        assert!(session.poll());
        assert_eq!(session.view_for(&bucket), ProbeView::Failed(ProbeFailure::MissingKeys));
    }

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
