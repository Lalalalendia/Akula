//! Native, explicitly user-invoked Yandex Object Storage key vault.
//!
//! Never serialize S3 secrets to AppConfig, SQLite, a user's knowledge base,
//! logs or clipboard. Windows input is through CredUI's masked password entry,
//! Other platforms intentionally fail closed until their own secure stores

use aquilum_core::settings::models::YandexS3Onboarding;
#[cfg(feature = "s3-pair-beta")]
mod imported;
#[cfg(feature = "s3-pair-beta")]
pub(crate) use imported::{save_profile_keys, with_profile_keys};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultError {
    InvalidBucket,
    InvalidKeys,
    MissingKeys,
    Unavailable,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved,
    Cancelled,
}

fn check_bucket(bucket: &str) -> Result<(), VaultError> {
    if bucket.is_empty() || !YandexS3Onboarding::valid_bucket(bucket) {
        return Err(VaultError::InvalidBucket);
    }
    Ok(())
}

/// The credential target is bucket-specific; switching settings to a different
/// bucket can never silently reuse the previous bucket's key pair.
pub fn target_name(bucket: &str) -> Result<String, VaultError> {
    check_bucket(bucket)?;
    Ok(format!("Akula/YandexObjectStorage/v1/{bucket}"))
}

pub fn present(bucket: &str) -> Result<bool, VaultError> {
    check_bucket(bucket)?;
    #[cfg(target_os = "windows")]
    {
        windows_vault::present(bucket)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err(VaultError::Unsupported)
    }
}

pub fn prompt_and_save(
    bucket: &str,
    title: &str,
    instructions: &str,
) -> Result<SaveOutcome, VaultError> {
    check_bucket(bucket)?;
    #[cfg(target_os = "windows")]
    {
        windows_vault::prompt_and_save(bucket, title, instructions)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (title, instructions);
        Err(VaultError::Unsupported)
    }
}

/// Access the saved bucket-specific keys only within a short-lived closure.
/// Intended for an explicitly invoked **background** read-only probe after
/// PR #408 is accepted. Never invoke this from settings render/GUI callbacks,
/// serialize the values, return them in closure output, or log SDK errors.
///
/// This function performs NO network operation or authorization itself.
#[allow(dead_code)]
pub(crate) fn with_stored_keys<R>(
    bucket: &str,
    action: impl FnOnce(&str, &str) -> R,
) -> Result<R, VaultError> {
    check_bucket(bucket)?;
    #[cfg(target_os = "windows")]
    {
        windows_vault::with_stored_keys(bucket, action)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = action;
        Err(VaultError::Unsupported)
    }
}

/// A point-in-time recheck after an explicit read-only network request.
/// No key bytes escape this helper; this does NOT guarantee that a key
/// cannot be replaced again after the check returns.
#[allow(dead_code)]
pub(crate) fn still_matches(bucket: &str, expected_id: &str, expected_secret: &str) -> bool {
    with_stored_keys(bucket, |id, secret| {
        id == expected_id && secret == expected_secret
    }).unwrap_or(false)
}

pub fn remove(bucket: &str) -> Result<(), VaultError> {
    check_bucket(bucket)?;
    #[cfg(target_os = "windows")]
    {
        windows_vault::remove(bucket)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err(VaultError::Unsupported)
    }
}

#[cfg(target_os = "windows")]
mod windows_vault {
    use super::{SaveOutcome, VaultError, target_name};
    use std::ffi::c_void;
    use zeroize::Zeroizing;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{GetLastError, ERROR_CANCELLED, ERROR_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredUIPromptForCredentialsW, CredWriteW, CREDENTIALW,
        CREDUI_INFOW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
        CREDUI_FLAGS_ALWAYS_SHOW_UI, CREDUI_FLAGS_DO_NOT_PERSIST,
        CREDUI_FLAGS_GENERIC_CREDENTIALS,
    };

    pub(super) fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn zero_terminated_len(text: &[u16]) -> usize {
        text.iter().position(|c| *c == 0).unwrap_or(text.len())
    }

    /// Windows allocates one credential buffer for CredReadW. Return it on
    /// *every* path, including corrupt credential values and callback panics.
    struct ReadCredential(*mut CREDENTIALW);
    impl Drop for ReadCredential {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CredFree(self.0.cast::<c_void>()) };
            }
        }
    }

    /// Decode only credentials written to Akula's versioned per-bucket target.
    /// Never expose any decoded values to a settings control or error string.
    pub(super) fn with_stored_keys<R>(
        bucket: &str,
        action: impl FnOnce(&str, &str) -> R,
    ) -> Result<R, VaultError> {
        let name = target_name(bucket)?;
        let target = wide(&name);
        let mut raw: *mut CREDENTIALW = std::ptr::null_mut();
        if unsafe { CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut raw) }.is_err() {
            return if unsafe { GetLastError() } == ERROR_NOT_FOUND {
                Err(VaultError::MissingKeys)
            } else {
                Err(VaultError::Unavailable)
            };
        }
        let allocation = ReadCredential(raw);
        if raw.is_null() {
            return Err(VaultError::Unavailable);
        }
        // CredReadW returns a Windows-owned NUL-terminated UserName. We bound
        // inspection and reject untrusted/non-ASCII values before allocating
        // application strings. This is not a substitute for WinAPI's validity.
        let stored = unsafe { &*allocation.0 };
        let username = stored.UserName.0;
        if username.is_null() || stored.CredentialBlob.is_null()
            || !(8..=255).contains(&(stored.CredentialBlobSize as usize))
        {
            return Err(VaultError::InvalidKeys);
        }
        let mut len = 0usize;
        while len <= 128 && unsafe { *username.add(len) } != 0 {
            len += 1;
        }
        if !(3..=128).contains(&len) {
            return Err(VaultError::InvalidKeys);
        }
        let wide_user = unsafe { std::slice::from_raw_parts(username, len) };
        if !wide_user.iter().all(|c| {
            *c < 128 && ((*c as u8).is_ascii_alphanumeric()
                || *c == u16::from(b'-') || *c == u16::from(b'_'))
        }) {
            return Err(VaultError::InvalidKeys);
        }
        let id = Zeroizing::new(
            String::from_utf16(wide_user).map_err(|_| VaultError::InvalidKeys)?
        );
        let secret_blob = Zeroizing::new(unsafe {
            std::slice::from_raw_parts(stored.CredentialBlob, stored.CredentialBlobSize as usize)
                .to_vec()
        });
        if !secret_blob.iter().all(u8::is_ascii_graphic) {
            return Err(VaultError::InvalidKeys);
        }
        // The UTF-8 conversion is a transient duplicate. Both copies are
        // zeroized on every path, including unwinding through the closure.
        let secret = Zeroizing::new(
            String::from_utf8(secret_blob.to_vec()).map_err(|_| VaultError::InvalidKeys)?
        );
        // Release the extra OS allocation before invoking user code.
        // No Win32 credential pointers escape this function.
        drop(allocation);
        Ok(action(&id, &secret))
    }

    pub(super) fn present(bucket: &str) -> Result<bool, VaultError> {
        let name = target_name(bucket)?;
        let target = wide(&name);
        let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
        // No credential bytes are copied into the app; the OS-allocated
        // structure is immediately returned to Windows with CredFree.
        let result = unsafe {
            CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut cred)
        };
        if result.is_err() {
            let code = unsafe { GetLastError() };
            return if code == ERROR_NOT_FOUND {
                Ok(false)
            } else {
                Err(VaultError::Unavailable)
            };
        }
        let allocated = ReadCredential(cred);
        if allocated.0.is_null() {
            return Err(VaultError::Unavailable);
        }
        Ok(true)
    }

    pub(super) fn remove(bucket: &str) -> Result<(), VaultError> {
        let name = target_name(bucket)?;
        let target = wide(&name);
        match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
            Ok(()) => Ok(()),
            Err(_) => {
                let code = unsafe { GetLastError() };
                if code == ERROR_NOT_FOUND { Ok(()) } else { Err(VaultError::Unavailable) }
            }
        }
    }

    /// Standard Windows credential dialog: first text field = Access Key ID,
    /// masked password field = Secret Access Key. Force Windows itself not to
    /// persist; only a validated pair is explicitly written by Akula below.
    pub(super) fn prompt_and_save(
        bucket: &str,
        title: &str,
        instructions: &str,
    ) -> Result<SaveOutcome, VaultError> {
        let name = target_name(bucket)?;
        let mut target = wide(&name);
        let caption = wide(title);
        let details = wide(instructions);
        let info = CREDUI_INFOW {
            cbSize: std::mem::size_of::<CREDUI_INFOW>() as u32,
            pszCaptionText: PCWSTR(caption.as_ptr()),
            pszMessageText: PCWSTR(details.as_ptr()),
            ..Default::default()
        };
        // Zeroizing overwrites buffers on every exit, including cancel and
        // any error. Do not create UI TextEditor buffers for these secrets.
        let mut user = Zeroizing::new(vec![0u16; 256]);
        let mut pass = Zeroizing::new(vec![0u16; 256]);
        let rc = unsafe {
            CredUIPromptForCredentialsW(
                Some(&info), PCWSTR(target.as_ptr()), None, 0,
                &mut user[..], &mut pass[..], None,
                CREDUI_FLAGS_GENERIC_CREDENTIALS
                    | CREDUI_FLAGS_ALWAYS_SHOW_UI
                    | CREDUI_FLAGS_DO_NOT_PERSIST,
            )
        };
        if rc == ERROR_CANCELLED {
            return Ok(SaveOutcome::Cancelled);
        }
        if rc != ERROR_SUCCESS {
            return Err(VaultError::Unavailable);
        }

        let user_len = zero_terminated_len(&user);
        let pass_len = zero_terminated_len(&pass);
        // This is only syntax validation; key validity/permissions are NOT
        // proven until the separate, user-approved read-only S3 probe.
        if !(3..=128).contains(&user_len)
            || !(8..=255).contains(&pass_len)
            || !user[..user_len].iter().all(|c| {
                *c < 128 && ((*c as u8).is_ascii_alphanumeric()
                    || *c == u16::from(b'-') || *c == u16::from(b'_'))
            })
        {
            return Err(VaultError::InvalidKeys);
        }
        let secret = Zeroizing::new(
            String::from_utf16(&pass[..pass_len]).map_err(|_| VaultError::InvalidKeys)?
        );
        if !secret.as_bytes().iter().all(u8::is_ascii_graphic) {
            return Err(VaultError::InvalidKeys);
        }
        let mut secret_bytes = Zeroizing::new(secret.as_bytes().to_vec());
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target.as_mut_ptr()),
            UserName: PWSTR(user.as_mut_ptr()),
            CredentialBlobSize: secret_bytes.len() as u32,
            CredentialBlob: secret_bytes.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        // Atomic Windows Credential Manager replace scoped to this bucket.
        // No conversion to AWS SDK capability and no implicit network probe.
        unsafe { CredWriteW(&credential, 0) }.map_err(|_| VaultError::Unavailable)?;
        Ok(SaveOutcome::Saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn targets_are_strict_and_bucket_scoped() {
        assert_ne!(target_name("bucket-one").unwrap(), target_name("bucket-two").unwrap());
        assert_eq!(target_name("bucket-one").unwrap(),
            "Akula/YandexObjectStorage/v1/bucket-one");
        for invalid in ["", "x", "bucket.with.dots", "https://example.invalid", "../steal"] {
            assert_eq!(target_name(invalid), Err(VaultError::InvalidBucket));
        }
    }
    // This is an actual Windows Credential Manager round-trip with strictly
    // disposable, non-Yandex test bytes. No CredUI prompt, network, or real
    // account key is involved, so it also runs on noninteractive CI runners.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_credential_manager_isolated_roundtrip() {
        use super::windows_vault::wide;
        use std::time::{SystemTime, UNIX_EPOCH};
        use windows::core::PWSTR;
        use windows::Win32::Security::Credentials::{
            CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
        };
        use zeroize::Zeroizing;

        // A per-run bucket name cannot collide with a user's ordinary
        // credentials. Never use the example/production bucket name.
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let bucket = format!("akula-test-{}-{now}", std::process::id());
        let target = target_name(&bucket).unwrap();
        struct RemoveOnDrop(String);
        impl Drop for RemoveOnDrop {
            fn drop(&mut self) { let _ = super::remove(&self.0); }
        }
        let _guard = RemoveOnDrop(bucket.clone());

        assert!(!present(&bucket).unwrap(), "new target unexpectedly already exists");

        let mut target_utf16 = wide(&target);
        let mut user_utf16 = wide("akula-test-access-id");
        let mut save_fake = |payload: &[u8]| {
            let mut secret = Zeroizing::new(payload.to_vec());
            let credential = CREDENTIALW {
                Type: CRED_TYPE_GENERIC,
                TargetName: PWSTR(target_utf16.as_mut_ptr()),
                UserName: PWSTR(user_utf16.as_mut_ptr()),
                CredentialBlobSize: secret.len() as u32,
                CredentialBlob: secret.as_mut_ptr(),
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                ..Default::default()
            };
            unsafe { CredWriteW(&credential, 0) }.expect("test-only write to Credential Manager");
        };

        save_fake(b"synthetic-test-secret-v1");
        assert!(still_matches(&bucket, "akula-test-access-id", "synthetic-test-secret-v1"));
        assert!(present(&bucket).unwrap());
        assert_eq!(with_stored_keys(&bucket, |id, secret| {
            id == "akula-test-access-id" && secret == "synthetic-test-secret-v1"
        }), Ok(true));
        assert!(!present("akula-test-unrelated-bucket").unwrap());
        assert_eq!(with_stored_keys("akula-test-unrelated-bucket", |_, _| ()),
            Err(VaultError::MissingKeys));

        // Updating the same bucket must retain the same target identity.
        save_fake(b"synthetic-test-secret-v2");
        assert!(!still_matches(&bucket, "akula-test-access-id", "synthetic-test-secret-v1"),
            "old S3 authorization must not survive a Windows credential overwrite");
        assert!(still_matches(&bucket, "akula-test-access-id", "synthetic-test-secret-v2"));
        assert_eq!(with_stored_keys(&bucket, |_, secret| {
            secret == "synthetic-test-secret-v2"
        }), Ok(true));
        // Corrupt stored bytes must fail closed: malformed legacy or
        // overwritten Credential Manager entries are never sent to an SDK.
        save_fake(b"non-graphic\nsecret");
        assert_eq!(with_stored_keys(&bucket, |_, _| ()),
            Err(VaultError::InvalidKeys));
        save_fake(b"synthetic-test-secret-v2");
        assert!(present(&bucket).unwrap());
        remove(&bucket).expect("test-only delete from Credential Manager");
        assert!(!present(&bucket).unwrap(), "delete must revoke local presence");
        assert!(!still_matches(&bucket, "akula-test-access-id", "synthetic-test-secret-v2"));
        assert_eq!(with_stored_keys(&bucket, |_, _| ()),
            Err(VaultError::MissingKeys));
        remove(&bucket).expect("repeated delete should be idempotent");
        assert_eq!(remove("not.valid"), Err(VaultError::InvalidBucket));
    }

    // This is the ACTUAL Microsoft CredUI dialog called by Akula, not a
    // credential store API substitute. A separate ephemeral Windows CI
    // process must first observe its masked password field and send Escape
    // through the OS. Never run on a developer's Windows account.
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "requires real Windows desktop and scripts/test-yandex-credui-cancel.ps1"]
    fn windows_credui_cancel_real_os_dialog_does_not_persist() {
        assert_eq!(std::env::var("RUNNER_ENVIRONMENT").as_deref(), Ok("github-hosted"));
        assert_eq!(std::env::var("RUNNER_OS").as_deref(), Ok("Windows"));
        assert_eq!(std::env::var("AKULA_CREDUI_CANCEL_CI").as_deref(), Ok("1"));
        use std::time::{SystemTime, UNIX_EPOCH};
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch").as_nanos();
        let bucket = format!("akula-credui-cancel-{}-{nonce}", std::process::id());
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = super::remove(&self.0);
            }
        }
        let _cleanup = Cleanup(bucket.clone());
        assert_eq!(present(&bucket), Ok(false), "synthetic target must not exist");
        let outcome = prompt_and_save(
            &bucket,
            "Akula CredUI Cancel QA",
            "This is an isolated test. Do not enter any real credentials. Cancel.",
        );
        assert_eq!(outcome, Ok(SaveOutcome::Cancelled),
            "Escape must cancel native Windows CredUI without storing keys");
        assert_eq!(present(&bucket), Ok(false),
            "Cancelling native Windows CredUI must not write any credentials");
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn unsupported_platform_never_prompts_or_writes_credentials() {
        assert_eq!(present("bucket-one"), Err(VaultError::Unsupported));
        assert_eq!(remove("bucket-one"), Err(VaultError::Unsupported));
        assert_eq!(with_stored_keys("bucket-one", |_, _| ()), Err(VaultError::Unsupported));
        assert_eq!(with_stored_keys("", |_, _| ()), Err(VaultError::InvalidBucket));
        assert_eq!(prompt_and_save("bucket-one", "title", "message"),
            Err(VaultError::Unsupported));
    }
}
