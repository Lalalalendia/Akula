//! An account-only OS credential scope; never confused with a Vault token.
//! This is deliberately independent of the signed Vault/file-sync authority.

use crate::{validation::valid_label, RefreshToken, SecretError};
use sha2::{Digest, Sha256};
use std::fmt;
#[cfg(feature = "system-store")]
use crate::validation::matches_readback;
use zeroize::Zeroizing;

const SERVICE: &str = "org.akula.dropbox.account-readonly.v1";

pub struct AccountCredentialScope {
    username: String,
}
impl fmt::Debug for AccountCredentialScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AccountCredentialScope(FINGERPRINTED)")
    }
}
impl AccountCredentialScope {
    /// All three fields come from the registered app and verified
    /// /users/get_current_account, never from a Dropbox file path.
    pub fn new(app: &str, account: &str, root: &str) -> Result<Self, SecretError> {
        let Some(suffix) = account.strip_prefix("dbid:") else {
            return Err(SecretError::InvalidScope);
        };
        if !valid_label(app, 128) || !valid_label(suffix, 128)
            || root.is_empty() || root.len() > 32
            || !root.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(SecretError::InvalidScope);
        }
        let mut digest = Sha256::new();
        digest.update(b"akula.dropbox.account.v1");
        for part in [app, account, root] {
            digest.update((part.len() as u64).to_le_bytes());
            digest.update(part.as_bytes());
        }
        Ok(Self { username: format!("account_{}", digest.finalize().iter().map(|byte| format!("{byte:02x}")).collect::<String>()) })
    }
}

/// Uses the actual host keyring. No disk, in-memory or plaintext fallback.
pub struct AccountCredentialStore {
    #[cfg(feature = "system-store")]
    lock: std::sync::Mutex<()>,
}
impl fmt::Debug for AccountCredentialStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AccountCredentialStore(REDACTED)")
    }
}
impl AccountCredentialStore {
    #[cfg(feature = "system-store")]
    pub fn system() -> Result<Self, SecretError> {
        Ok(Self { lock: std::sync::Mutex::new(()) })
    }

    #[cfg(not(feature = "system-store"))]
    pub fn system() -> Result<Self, SecretError> {
        Err(SecretError::StoreUnavailable)
    }

    #[cfg(feature = "system-store")]
    fn entry(&self, scope: &AccountCredentialScope) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(SERVICE, &scope.username)
            .map_err(|_| SecretError::StoreUnavailable)
    }

    /// A successful OS write is trusted only after exact read-back.
    pub fn save_refresh(
        &self, scope: &AccountCredentialScope, token: &RefreshToken,
    ) -> Result<(), SecretError> {
        #[cfg(feature = "system-store")]
        {
            let _guard = self.lock.lock().map_err(|_| SecretError::StoreFailure)?;
            let entry = self.entry(scope)?;
            entry.set_password(token.as_str()).map_err(|_| SecretError::StoreFailure)?;
            let readback = Zeroizing::new(
                entry.get_password().map_err(|_| SecretError::VerificationFailed)?);
            if !matches_readback(token.as_str().as_bytes(), readback.as_bytes()) {
                return Err(SecretError::VerificationFailed);
            }
            Ok(())
        }
        #[cfg(not(feature = "system-store"))]
        {
            let _ = (scope, token);
            Err(SecretError::StoreUnavailable)
        }
    }

    pub fn load_refresh(
        &self, scope: &AccountCredentialScope,
    ) -> Result<Option<RefreshToken>, SecretError> {
        #[cfg(feature = "system-store")]
        {
            let _guard = self.lock.lock().map_err(|_| SecretError::StoreFailure)?;
            match self.entry(scope)?.get_password() {
                Ok(value) => RefreshToken::new(value).map(Some),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(_) => Err(SecretError::StoreFailure),
            }
        }
        #[cfg(not(feature = "system-store"))]
        {
            let _ = scope;
            Err(SecretError::StoreUnavailable)
        }
    }

    /// Forgetting local consent is not remote OAuth revocation.
    pub fn forget_local(&self, scope: &AccountCredentialScope) -> Result<(), SecretError> {
        #[cfg(feature = "system-store")]
        {
            let _guard = self.lock.lock().map_err(|_| SecretError::StoreFailure)?;
            match self.entry(scope)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(_) => Err(SecretError::StoreFailure),
            }
        }
        #[cfg(not(feature = "system-store"))]
        {
            let _ = scope;
            Err(SecretError::StoreUnavailable)
        }
    }
}

#[cfg(test)]
#[path = "account/tests.rs"]
mod tests;
