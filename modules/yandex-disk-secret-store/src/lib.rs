#![forbid(unsafe_code)]
//! Yandex Disk refresh-token storage using the host operating system.
//! No plaintext file, SQLite, JSON, environment-variable, or in-memory
//! fallback in production. Missing OS secret service always fails closed.
//! This crate has no network calls and no File Gate or Sync Core authority.

use sha2::{Digest, Sha256};
use std::fmt;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

const SERVICE: &str = "org.akula.yandex-disk.app-folder.v1";
const MAX_TOKEN_BYTES: usize = 4096;

#[cfg(all(feature = "system-store", not(any(
    target_os = "linux", target_os = "macos", target_os = "windows"
))))]
compile_error!("No approved native secret store backend for this operating system");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretError {
    InvalidScope,
    InvalidToken,
    StoreUnavailable,
    StoreFailure,
    VerificationFailed,
}

fn valid_uuid(raw: &str) -> bool {
    let b = raw.as_bytes();
    b.len() == 36 && b.iter().enumerate().all(|(i, ch)| {
        if [8, 13, 18, 23].contains(&i) { *ch == b'-' }
        else { ch.is_ascii_digit() || (b'a'..=b'f').contains(ch) }
    })
}

fn valid_label(label: &str, max: usize) -> bool {
    !label.is_empty() && label.len() <= max &&
        label.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

/// OS-keystore scope for a verified Yandex ID account and one Akula Vault.
/// Require the subject ID and OAuth client ID from a same-session HTTPS
/// identity check, not a login hint or account display name. This constructor
/// rejects malformed labels but cannot independently prove authorization.
/// It contains no bearer token. Key names expose only a SHA-256 fingerprint.
pub struct YandexCredentialScope {
    username: String,
}

impl fmt::Debug for YandexCredentialScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("YandexCredentialScope(FINGERPRINTED)")
    }
}

impl YandexCredentialScope {
    pub fn new(
        account_id: &str, vault_id: &str,
        profile_id: &str, oauth_app_key: &str,
    ) -> Result<Self, SecretError> {
        if account_id.is_empty() || account_id.len() > 64
            || !account_id.bytes().all(|b| b.is_ascii_digit())
            || !valid_uuid(vault_id)
            || !valid_label(profile_id, 64)
            || !valid_label(oauth_app_key, 128)
        { return Err(SecretError::InvalidScope); }

        let mut hash = Sha256::new();
        hash.update(b"AKULA:YANDEX-DISK:OS-VAULT:V1");
        for component in [account_id, oauth_app_key, vault_id, profile_id] {
            hash.update((component.len() as u64).to_le_bytes());
            hash.update(component.as_bytes());
        }
        Ok(Self { username: format!("v1_{}", hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect::<String>()) })
    }

    pub fn is_same_scope(&self, other: &Self) -> bool {
        self.username == other.username
    }
    fn username(&self) -> &str { &self.username }
}

/// No Clone/Display/Serialize; the token is wiped when dropped.
pub struct YandexRefreshToken(Zeroizing<String>);

impl fmt::Debug for YandexRefreshToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("YandexRefreshToken(REDACTED)")
    }
}

impl YandexRefreshToken {
    pub fn new(text: String) -> Result<Self, SecretError> {
        Self::from_protected(Zeroizing::new(text))
    }
    fn from_protected(text: Zeroizing<String>) -> Result<Self, SecretError> {
        if text.is_empty() || text.len() > MAX_TOKEN_BYTES ||
            !text.bytes().all(|b| b.is_ascii_graphic())
        { return Err(SecretError::InvalidToken); }
        Ok(Self(text))
    }
    pub fn as_str(&self) -> &str { &self.0 }
}

trait Backend: Send + Sync {
    fn put(&self, username: &str, token: &str) -> Result<(), SecretError>;
    fn get(&self, username: &str) -> Result<Option<Zeroizing<String>>, SecretError>;
    fn delete(&self, username: &str) -> Result<(), SecretError>;
}

/// Callers can construct only the real native backend; a test-only in-memory
/// backend exists behind cfg(test), and cannot be used by the application.
pub struct YandexCredentialVault {
    backend: Box<dyn Backend>,
}

impl fmt::Debug for YandexCredentialVault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("YandexCredentialVault(REDACTED)")
    }
}

impl YandexCredentialVault {
    /// Never falls back to an in-memory/mock provider or a plaintext file.
    #[cfg(feature = "system-store")]
    pub fn system() -> Result<Self, SecretError> {
        Ok(Self { backend: Box::new(NativeBackend::new()) })
    }

    /// Without a compiled native backend, reject all credential operations.
    #[cfg(not(feature = "system-store"))]
    pub fn system() -> Result<Self, SecretError> {
        Err(SecretError::StoreUnavailable)
    }

    /// An OS store returning success is not enough: read the exact entry
    /// back before reporting the account as connected.
    pub fn save_refresh(&self, scope: &YandexCredentialScope, token: &YandexRefreshToken)
        -> Result<(), SecretError>
    {
        self.backend.put(scope.username(), token.as_str())?;
        let returned = self.backend.get(scope.username())?
            .ok_or(SecretError::VerificationFailed)?;
        let b = returned.as_bytes();
        let a = token.as_str().as_bytes();
        if a.len() != b.len() || !bool::from(a.ct_eq(b)) {
            return Err(SecretError::VerificationFailed);
        }
        Ok(())
    }

    pub fn load_refresh(&self, scope: &YandexCredentialScope)
        -> Result<Option<YandexRefreshToken>, SecretError>
    {
        let value = self.backend.get(scope.username())?;
        value.map(YandexRefreshToken::from_protected).transpose()
    }

}

#[cfg(feature = "system-store")]
#[path = "../../secret-store-common/src/native_backend.rs"]
mod native_backend;
#[cfg(feature = "system-store")]
use native_backend::NativeBackend;

mod lifecycle;
pub use lifecycle::StoredRefreshPresence;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
