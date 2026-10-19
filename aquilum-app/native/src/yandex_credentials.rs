//! Native, explicitly user-invoked Yandex Object Storage key vault.
//!
//! Never serialize S3 secrets to AppConfig, SQLite, a user's knowledge base,
//! logs or clipboard. Windows input is through CredUI's masked password entry,
//! followed by a separate CredWriteW with user-logon-scoped persistence.
//! No network activity, S3 authorizations or trust decisions are performed.
//! Other platforms intentionally fail closed until their own secure stores
//! and modal secret-entry controls pass native CI.

use aquilum_core::settings::models::YandexS3Onboarding;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultError {
    InvalidBucket,
    InvalidKeys,
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

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn zero_terminated_len(text: &[u16]) -> usize {
        text.iter().position(|c| *c == 0).unwrap_or(text.len())
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
        if cred.is_null() {
            return Err(VaultError::Unavailable);
        }
        unsafe { CredFree(cred.cast::<c_void>()) };
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
        assert!(present(&bucket).unwrap());
        assert!(!present("akula-test-unrelated-bucket").unwrap());

        // Updating the same bucket must retain the same target identity.
        save_fake(b"synthetic-test-secret-v2");
        assert!(present(&bucket).unwrap());
        remove(&bucket).expect("test-only delete from Credential Manager");
        assert!(!present(&bucket).unwrap(), "delete must revoke local presence");
        remove(&bucket).expect("repeated delete should be idempotent");
        assert_eq!(remove("not.valid"), Err(VaultError::InvalidBucket));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn unsupported_platform_never_prompts_or_writes_credentials() {
        assert_eq!(present("bucket-one"), Err(VaultError::Unsupported));
        assert_eq!(remove("bucket-one"), Err(VaultError::Unsupported));
        assert_eq!(prompt_and_save("bucket-one", "title", "message"),
            Err(VaultError::Unsupported));
    }
}
