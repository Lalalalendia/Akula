//! An account-only OS credential scope; never confused with a Vault token.
//! This is deliberately independent of the signed Vault/file-sync authority.

use crate::{RefreshToken, SecretError};
use sha2::{Digest, Sha256};
use std::fmt;
use subtle::ConstantTimeEq;
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
        let label = |s: &str, max: usize| {
            !s.is_empty() && s.len() <= max && s.bytes().all(|b|
                b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        };
        if !label(app, 128) || !label(suffix, 128)
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
            let a = token.as_str().as_bytes();
            let b = readback.as_bytes();
            if a.len() != b.len() || !bool::from(a.ct_eq(b)) {
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
mod tests {
    use super::*;

    #[test]
    fn account_identity_is_separate_from_vault_and_other_apps() {
        let a = AccountCredentialScope::new("akula_app", "dbid:alice", "12345").unwrap();
        let b = AccountCredentialScope::new("akula_app", "dbid:alice", "12345").unwrap();
        assert_eq!(a.username, b.username);
        for (app, account, root) in [
            ("other_app", "dbid:alice", "12345"),
            ("akula_app", "dbid:bob", "12345"),
            ("akula_app", "dbid:alice", "55555"),
        ] {
            assert_ne!(a.username,
                AccountCredentialScope::new(app, account, root).unwrap().username);
        }
        assert!(!format!("{a:?}").contains("alice"));
        assert!(!a.username.contains("alice"));
        assert!(!a.username.contains("akula_app"));
        assert_eq!(SERVICE, "org.akula.dropbox.account-readonly.v1");
    }

    #[test]
    fn account_scope_rejects_control_chars_paths_and_non_numeric_roots() {
        for (app, account, root) in [
            ("", "dbid:alice", "123"), ("bad/app", "dbid:alice", "123"),
            ("akula_app", "alice", "123"), ("akula_app", "dbid:bad/account", "123"),
            ("akula_app", "dbid:alice", ""), ("akula_app", "dbid:alice", "../1"),
        ] {
            assert!(AccountCredentialScope::new(app, account, root).is_err());
        }
    }

    #[test]
    fn without_real_os_store_tokens_cannot_be_persisted_in_a_mock() {
        if !cfg!(feature = "system-store") {
            assert!(matches!(AccountCredentialStore::system(),
                Err(SecretError::StoreUnavailable)));
        }
    }
}
