//! Device-local S3 pairing credentials. A pairing vault is NEVER addressed
//! by bucket alone: different provider/endpoint/prefix/VaultId/genesis pins
//! must resolve to different OS Credential Manager targets.
//!
//! This does not change the legacy bucket-only Yandex connection preview.
//! First-device setup can read its bucket-only *input* once, then copies the
//! selected key pair into this strictly profile-bound target.

use aquilum_core::sync::s3_pair_profile::S3PairProfile;
use sha2::{Digest, Sha256};
use super::VaultError;

pub(crate) fn profile_target_name(profile: &S3PairProfile) -> Result<String, VaultError> {
    profile.validate().map_err(|_| VaultError::InvalidBucket)?;
    // Non-secret, deterministic canonical serialization of the fixed
    // S3PairProfile V1 structure; a new schema must use a new target version.
    let json = serde_json::to_vec(profile).map_err(|_| VaultError::InvalidBucket)?;
    let digest = Sha256::digest(json);
    let hex = digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    Ok(format!("Akula/S3Pair/v1/{hex}"))
}

#[cfg(target_os = "windows")]
mod wincred {
    use super::*;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND};
    use windows::Win32::Security::Credentials::{
        CredFree, CredReadW, CredWriteW, CREDENTIALW,
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    };
    use std::ffi::c_void;
    use zeroize::Zeroizing;

    struct OwnedCredential(*mut CREDENTIALW);
    impl Drop for OwnedCredential {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: returned by CredReadW, freed once here.
                unsafe { CredFree(self.0.cast::<c_void>()); }
            }
        }
    }

    pub(crate) fn with_profile_keys<R>(
        profile: &S3PairProfile,
        action: impl FnOnce(&str, &str) -> R,
    ) -> Result<R, VaultError> {
        let target = profile_target_name(profile)?;
        let wide: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
        let mut raw: *mut CREDENTIALW = std::ptr::null_mut();
        if unsafe { CredReadW(PCWSTR(wide.as_ptr()), CRED_TYPE_GENERIC, None, &mut raw) }
            .is_err()
        {
            return if unsafe { GetLastError() } == ERROR_NOT_FOUND {
                Err(VaultError::MissingKeys)
            } else {
                Err(VaultError::Unavailable)
            };
        }
        let guard = OwnedCredential(raw);
        if guard.0.is_null() {
            return Err(VaultError::Unavailable);
        }
        // OS pointers and secret bytes stay inside this short-lived worker.
        let value = unsafe { &*guard.0 };
        if value.UserName.0.is_null() || value.CredentialBlob.is_null()
            || !(8..=255).contains(&(value.CredentialBlobSize as usize))
        {
            return Err(VaultError::InvalidKeys);
        }
        let user_ptr = value.UserName.0;
        let mut len = 0;
        while len <= 128 && unsafe { *user_ptr.add(len) } != 0 {
            len += 1;
        }
        if !(3..=128).contains(&len) {
            return Err(VaultError::InvalidKeys);
        }
        let user_bytes = unsafe { std::slice::from_raw_parts(user_ptr, len) };
        if !user_bytes.iter().all(|x| *x < 128
            && ((*x as u8).is_ascii_alphanumeric() || *x == 45 || *x == 95))
        {
            return Err(VaultError::InvalidKeys);
        }
        let id = Zeroizing::new(
            String::from_utf16(user_bytes).map_err(|_| VaultError::InvalidKeys)?
        );
        let secret_bytes = Zeroizing::new(unsafe {
            std::slice::from_raw_parts(value.CredentialBlob, value.CredentialBlobSize as usize)
                .to_vec()
        });
        if !secret_bytes.iter().all(u8::is_ascii_graphic) {
            return Err(VaultError::InvalidKeys);
        }
        let secret = Zeroizing::new(
            String::from_utf8(secret_bytes.to_vec()).map_err(|_| VaultError::InvalidKeys)?
        );
        drop(guard);
        Ok(action(&id, &secret))
    }

    pub(crate) fn save_profile_keys(
        profile: &S3PairProfile, id: &str, secret: &str,
    ) -> Result<(), VaultError> {
        if !(3..=128).contains(&id.len())
            || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || !(8..=255).contains(&secret.len())
            || !secret.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(VaultError::InvalidKeys);
        }
        // Never overwrite another key for the exact same remote identity.
        // The OS has no compare-and-swap CredWriteW; any concurrent rotation
        // is detected by postwrite readback, not claimed as atomic revocation.
        match with_profile_keys(profile, |old_id, old_secret| {
            old_id == id && old_secret == secret
        }) {
            Ok(true) => return Ok(()),
            Ok(false) => return Err(VaultError::InvalidKeys),
            Err(VaultError::MissingKeys) => {}
            Err(error) => return Err(error),
        }
        let target = profile_target_name(profile)?;
        let mut target_w: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
        let mut user_w: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
        let mut secret_bytes = Zeroizing::new(secret.as_bytes().to_vec());
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target_w.as_mut_ptr()),
            UserName: PWSTR(user_w.as_mut_ptr()),
            CredentialBlobSize: secret_bytes.len() as u32,
            CredentialBlob: secret_bytes.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        unsafe { CredWriteW(&credential, 0) }.map_err(|_| VaultError::Unavailable)?;
        if with_profile_keys(profile, |saved_id, saved_secret| {
            saved_id == id && saved_secret == secret
        })? {
            Ok(())
        } else {
            Err(VaultError::InvalidKeys)
        }
    }

    #[cfg(test)]
    pub(super) fn remove_test_profile_keys(profile: &S3PairProfile) {
        use windows::Win32::Security::Credentials::CredDeleteW;
        if let Ok(target) = profile_target_name(profile) {
            let wide: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
            let _ = unsafe { CredDeleteW(PCWSTR(wide.as_ptr()), CRED_TYPE_GENERIC, None) };
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) use wincred::{save_profile_keys, with_profile_keys};

#[cfg(not(target_os = "windows"))]
pub(crate) fn save_profile_keys(
    _profile: &S3PairProfile, _id: &str, _secret: &str,
) -> Result<(), VaultError> {
    Err(VaultError::Unsupported)
}
#[cfg(not(target_os = "windows"))]
pub(crate) fn with_profile_keys<R>(
    _profile: &S3PairProfile, _action: impl FnOnce(&str, &str) -> R,
) -> Result<R, VaultError> {
    Err(VaultError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aquilum_core::sync::VaultId;

    #[test]
    fn scope_includes_genesis_vault_prefix_and_provider() {
        let original = S3PairProfile::yandex_first_device(
            "akula-test-scope", VaultId::new(), "a".repeat(64),
        );
        let first = profile_target_name(&original).unwrap();
        assert!(first.starts_with("Akula/S3Pair/v1/"));
        assert!(!first.contains("akula-test-scope"));
        assert_eq!(first, profile_target_name(&original).unwrap());
        let mut change = original.clone();
        change.cloud_vault_id = VaultId::new();
        assert_ne!(first, profile_target_name(&change).unwrap());
        change = original.clone();
        change.genesis_fingerprint = "b".repeat(64);
        assert_ne!(first, profile_target_name(&change).unwrap());
        change = original.clone();
        change.prefix = "akula-sync/other".into();
        assert_ne!(first, profile_target_name(&change).unwrap());
        change = original.clone();
        change.provider = "minio".into();
        assert_ne!(first, profile_target_name(&change).unwrap());
        change = original.clone();
        change.bucket = "akula-second-scope".into();
        assert_ne!(first, profile_target_name(&change).unwrap());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn real_windows_profile_credential_roundtrip_and_collision_refusal() {
        let profile = S3PairProfile::yandex_first_device(
            "akula-test-profile", VaultId::new(), "a".repeat(64),
        );
        struct Cleanup(S3PairProfile);
        impl Drop for Cleanup {
            fn drop(&mut self) { wincred::remove_test_profile_keys(&self.0); }
        }
        let _cleanup = Cleanup(profile.clone());
        assert_eq!(
            with_profile_keys(&profile, |_, _| ()), Err(VaultError::MissingKeys)
        );
        save_profile_keys(&profile, "akula-test-access", "fake-test-secret-12345").unwrap();
        save_profile_keys(&profile, "akula-test-access", "fake-test-secret-12345").unwrap();
        assert!(with_profile_keys(&profile, |id, secret| {
            id == "akula-test-access" && secret == "fake-test-secret-12345"
        }).unwrap());
        assert_eq!(
            save_profile_keys(&profile, "akula-test-access", "different-fake-secret-123"),
            Err(VaultError::InvalidKeys),
        );
        let different = S3PairProfile::yandex_first_device(
            "akula-test-profile", VaultId::new(), "a".repeat(64),
        );
        assert_eq!(
            with_profile_keys(&different, |_, _| ()), Err(VaultError::MissingKeys)
        );
    }
}
