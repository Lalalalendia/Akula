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
