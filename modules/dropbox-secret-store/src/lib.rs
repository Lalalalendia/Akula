#![forbid(unsafe_code)]
//! Dropbox refresh-token storage using the host operating system.
//! No plaintext file, SQLite, JSON, environment-variable, or in-memory
//! fallback in production. Missing OS secret service always fails closed.
//! This crate has no network calls and no File Gate or Sync Core authority.

pub mod account; use sha2::{Digest, Sha256};
use std::fmt;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

const SERVICE: &str = "org.akula.dropbox.readonly.v1";
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

/// The identity of a read-only credential in one Akula Vault Sync profile.
/// The Dropbox OAuth app key is REQUIRED: different App Folder applications
/// must not read or overwrite each other's refresh tokens, even when they
/// belong to the same Dropbox account and the same local Akula vault.
/// It contains no bearer token. Key names expose only a SHA-256 fingerprint.
pub struct CredentialScope {
    username: String,
}

impl fmt::Debug for CredentialScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CredentialScope(FINGERPRINTED)")
    }
}

impl CredentialScope {
    pub fn new(
        account_id: &str, root_namespace: &str, vault_id: &str,
        profile_id: &str, oauth_app_key: &str,
    ) -> Result<Self, SecretError> {
        let account = account_id.strip_prefix("dbid:").ok_or(SecretError::InvalidScope)?;
        if !valid_label(account, 128)
            || root_namespace.is_empty() || root_namespace.len() > 32
            || !root_namespace.bytes().all(|x| x.is_ascii_digit())
            || !valid_uuid(vault_id)
            || !valid_label(profile_id, 64)
            || !valid_label(oauth_app_key, 128)
        { return Err(SecretError::InvalidScope); }

        let mut hash = Sha256::new();
        for component in [account_id, root_namespace, vault_id, profile_id, oauth_app_key] {
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
pub struct RefreshToken(Zeroizing<String>);

impl fmt::Debug for RefreshToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RefreshToken(REDACTED)")
    }
}

impl RefreshToken {
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
pub struct CredentialVault {
    backend: Box<dyn Backend>,
}

impl fmt::Debug for CredentialVault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CredentialVault(REDACTED)")
    }
}

impl CredentialVault {
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
    pub fn save_refresh(&self, scope: &CredentialScope, token: &RefreshToken)
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

    pub fn load_refresh(&self, scope: &CredentialScope)
        -> Result<Option<RefreshToken>, SecretError>
    {
        let value = self.backend.get(scope.username())?;
        value.map(RefreshToken::from_protected).transpose()
    }

    /// Local forgetting does not revoke the token remotely at Dropbox.
    /// The UI must distinguish "forget on this device" from "revoke access".
    pub fn forget_local(&self, scope: &CredentialScope) -> Result<(), SecretError> {
        self.backend.delete(scope.username())
    }
}

#[cfg(feature = "system-store")]
struct NativeBackend {
    /// Serialize accesses inside one application instance (Windows does
    /// not guarantee ordering for writes to one credential).
    operations: std::sync::Mutex<()>,
}

#[cfg(feature = "system-store")]
impl NativeBackend {
    fn new() -> Self { Self { operations: std::sync::Mutex::new(()) } }

    fn entry(&self, username: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(SERVICE, username).map_err(|_| SecretError::StoreUnavailable)
    }
}

#[cfg(feature = "system-store")]
impl Backend for NativeBackend {
    fn put(&self, username: &str, token: &str) -> Result<(), SecretError> {
        let _lock = self.operations.lock().map_err(|_| SecretError::StoreFailure)?;
        self.entry(username)?.set_password(token)
            .map_err(|_| SecretError::StoreFailure)
    }

    fn get(&self, username: &str) -> Result<Option<Zeroizing<String>>, SecretError> {
        let _lock = self.operations.lock().map_err(|_| SecretError::StoreFailure)?;
        match self.entry(username)?.get_password() {
            Ok(value) => Ok(Some(Zeroizing::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(SecretError::StoreFailure),
        }
    }

    fn delete(&self, username: &str) -> Result<(), SecretError> {
        let _lock = self.operations.lock().map_err(|_| SecretError::StoreFailure)?;
        match self.entry(username)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(SecretError::StoreFailure),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, sync::Mutex};

    #[derive(Default)]
    struct MemoryBackend {
        entries: Mutex<HashMap<String, Zeroizing<String>>>,
        deny: bool,
        tamper: bool,
    }

    impl Backend for MemoryBackend {
        fn put(&self, name: &str, value: &str) -> Result<(), SecretError> {
            if self.deny { return Err(SecretError::StoreFailure); }
            self.entries.lock().unwrap().insert(name.into(), Zeroizing::new(
                if self.tamper { "changed-token" } else { value }.into()
            ));
            Ok(())
        }
        fn get(&self, name: &str) -> Result<Option<Zeroizing<String>>, SecretError> {
            if self.deny { return Err(SecretError::StoreFailure); }
            Ok(self.entries.lock().unwrap().get(name)
                .map(|x| Zeroizing::new(x.to_string())))
        }
        fn delete(&self, name: &str) -> Result<(), SecretError> {
            if self.deny { return Err(SecretError::StoreFailure); }
            self.entries.lock().unwrap().remove(name);
            Ok(())
        }
    }

    fn scope(account: &str, vault: &str) -> CredentialScope {
        CredentialScope::new(account, "123456", vault, "default", "vault_sync_app").unwrap()
    }
    const VAULT: &str = "12345678-1234-1234-1234-123456789abc";
    fn test_vault() -> CredentialVault {
        CredentialVault { backend: Box::<MemoryBackend>::default() }
    }

    #[test]
    fn scope_identity_is_stable_and_strictly_partitioned() {
        let a = scope("dbid:alice", VAULT);
        // Golden fingerprint from SHA-256 0.10, preserved for existing OS keyring entries.
        assert_eq!(a.username(),
            "v1_321363b84477819caf80b08e30a75e6ac88ae64443ea3e488294088349e804da");
        let copy = scope("dbid:alice", VAULT);
        assert!(a.is_same_scope(&copy));
        assert!(!a.is_same_scope(&scope("dbid:bob", VAULT)));
        assert!(!a.is_same_scope(&scope("dbid:alice",
            "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")));
        assert!(!a.is_same_scope(&CredentialScope::new(
            "dbid:alice", "654321", VAULT, "default", "vault_sync_app").unwrap()));
        assert!(!a.is_same_scope(&CredentialScope::new(
            "dbid:alice", "123456", VAULT, "other", "vault_sync_app").unwrap()));
        assert!(!a.is_same_scope(&CredentialScope::new(
            "dbid:alice", "123456", VAULT, "default", "other_dropbox_app").unwrap()));
        assert!(!a.username().contains("vault_sync_app"));
        assert!(!a.username().contains("alice"));
        assert!(!a.username().contains(VAULT));
        assert!(!format!("{a:?}").contains(VAULT));
    }

    #[test]
    fn oauth_apps_in_same_dropbox_account_cannot_share_a_refresh_token() {
        let vault = test_vault();
        let primary = scope("dbid:alice", VAULT);
        let dropbox_drop = CredentialScope::new(
            "dbid:alice", "123456", VAULT, "default", "drop_app").unwrap();
        vault.save_refresh(&primary, &RefreshToken::new("readonly-refresh".into()).unwrap()).unwrap();
        assert!(vault.load_refresh(&dropbox_drop).unwrap().is_none());
        vault.save_refresh(&dropbox_drop, &RefreshToken::new("other-app-refresh".into()).unwrap()).unwrap();
        vault.forget_local(&dropbox_drop).unwrap();
        assert_eq!(vault.load_refresh(&primary).unwrap().unwrap().as_str(), "readonly-refresh");
    }

    #[test]
    fn store_load_rotate_and_forget_only_exact_scope() {
        let v = test_vault();
        let a = scope("dbid:alice", VAULT);
        let b = scope("dbid:bob", VAULT);
        let token = RefreshToken::new("first-refresh".into()).unwrap();
        v.save_refresh(&a, &token).unwrap();
        assert_eq!(v.load_refresh(&a).unwrap().unwrap().as_str(), "first-refresh");
        assert!(v.load_refresh(&b).unwrap().is_none());
        v.save_refresh(&b, &RefreshToken::new("bob-refresh".into()).unwrap()).unwrap();
        v.save_refresh(&a, &RefreshToken::new("rotated-refresh".into()).unwrap()).unwrap();
        assert_eq!(v.load_refresh(&a).unwrap().unwrap().as_str(), "rotated-refresh");
        v.forget_local(&a).unwrap();
        assert!(v.load_refresh(&a).unwrap().is_none());
        assert_eq!(v.load_refresh(&b).unwrap().unwrap().as_str(), "bob-refresh");
    }

    #[test]
    fn no_plaintext_or_secret_in_debug_output() {
        let v = test_vault();
        let token = RefreshToken::new("top-secret-value".into()).unwrap();
        assert_eq!(format!("{token:?}"), "RefreshToken(REDACTED)");
        assert_eq!(format!("{v:?}"), "CredentialVault(REDACTED)");
        assert!(!format!("{token:?}").contains("top-secret-value"));
    }

    #[test]
    fn invalid_scopes_and_secrets_are_rejected() {
        for (account, root, vault, profile, app) in [
            ("alice", "123", VAULT, "default", "vault_sync_app"),
            ("dbid:alice", "../bad", VAULT, "default", "vault_sync_app"),
            ("dbid:alice", "123", "bad", "default", "vault_sync_app"),
            ("dbid:alice", "123", VAULT, "../bad", "vault_sync_app"),
            ("dbid:alice", "123", VAULT, "default", ""),
            ("dbid:alice", "123", VAULT, "default", "../different_app"),
        ] {
            assert!(matches!(CredentialScope::new(account, root, vault, profile, app),
                Err(SecretError::InvalidScope)));
        }
        assert!(matches!(RefreshToken::new("".into()), Err(SecretError::InvalidToken)));
        assert!(matches!(RefreshToken::new("hi\nsecret".into()),
            Err(SecretError::InvalidToken)));
    }

    #[test]
    fn unavailable_os_store_and_mismatched_readback_fail_closed() {
        let a = scope("dbid:alice", VAULT);
        let t = RefreshToken::new("good-token".into()).unwrap();
        let v = CredentialVault { backend: Box::new(MemoryBackend {
            deny: true, ..Default::default()
        }) };
        assert_eq!(v.save_refresh(&a, &t), Err(SecretError::StoreFailure));
        assert!(matches!(v.load_refresh(&a), Err(SecretError::StoreFailure)));
        let fake = CredentialVault { backend: Box::new(MemoryBackend {
            tamper: true, ..Default::default()
        }) };
        assert_eq!(fake.save_refresh(&a, &t), Err(SecretError::VerificationFailed));
    }

    #[test]
    fn no_system_store_build_never_uses_mock_for_production() {
        if !cfg!(feature = "system-store") {
            assert!(matches!(CredentialVault::system(),
                Err(SecretError::StoreUnavailable)));
        }
        assert_eq!(SERVICE, "org.akula.dropbox.readonly.v1");
    }
}
