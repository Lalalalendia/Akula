//! Import one scope-checked S3 key pair without a second manual key entry.
//! Windows owns the encrypted credential persistence; JSON must never
//! contain an Access Key ID or Secret Access Key.

use super::{target_name, VaultError};
use zeroize::Zeroizing;

#[cfg(target_os = "windows")]
pub(crate) fn save_imported_keys(
    bucket: &str,
    id: &str,
    secret: &str,
) -> Result<(), VaultError> {
    use windows::core::PWSTR;
    use windows::Win32::Security::Credentials::{
        CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    };
    if !(3..=128).contains(&id.len())
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        || !(8..=255).contains(&secret.len())
        || !secret.bytes().all(|b| b.is_ascii_graphic())
    {
        return Err(VaultError::InvalidKeys);
    }
    let target = target_name(bucket)?;
    // Never overwrite another live profile's key pair automatically.
    if super::present(bucket)? {
        let identical = super::with_stored_keys(bucket, |old_id, old_secret| {
            old_id == id && old_secret == secret
        })?;
        return if identical { Ok(()) } else { Err(VaultError::InvalidKeys) };
    }
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
    // Post-write readback catches incomplete/corrupt imports. No secret
    // reaches GUI status or ordinary AppConfig.
    let match_saved = super::with_stored_keys(bucket, |stored_id, stored_secret| {
        stored_id == id && stored_secret == secret
    })?;
    if match_saved { Ok(()) } else { Err(VaultError::Unavailable) }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn save_imported_keys(
    _bucket: &str,
    _id: &str,
    _secret: &str,
) -> Result<(), VaultError> {
    Err(VaultError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_bucket_does_not_make_a_credential_target() {
        assert!(matches!(target_name("bad bucket"), Err(VaultError::InvalidBucket)));
    }
}
