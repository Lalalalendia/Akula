use super::*;
use std::{collections::HashMap, sync::Mutex};

const VAULT: &str = "12345678-1234-1234-1234-123456789abc";
const APP: &str = "4760187d81bc4b7799476b42b5103713";

#[derive(Default)]
struct MemoryBackend {
    tokens: Mutex<HashMap<String, Zeroizing<String>>>,
    fail: bool,
    corrupt: bool,
}

impl Backend for MemoryBackend {
    fn put(&self, username: &str, token: &str) -> Result<(), SecretError> {
        if self.fail { return Err(SecretError::StoreFailure); }
        let value = if self.corrupt { "wrong-token" } else { token };
        self.tokens.lock().unwrap().insert(username.to_owned(), Zeroizing::new(value.to_owned()));
        Ok(())
    }

    fn get(&self, username: &str) -> Result<Option<Zeroizing<String>>, SecretError> {
        if self.fail { return Err(SecretError::StoreFailure); }
        Ok(self.tokens.lock().unwrap().get(username)
            .map(|value| Zeroizing::new(value.to_string())))
    }

    fn delete(&self, username: &str) -> Result<(), SecretError> {
        if self.fail { return Err(SecretError::StoreFailure); }
        self.tokens.lock().unwrap().remove(username);
        Ok(())
    }
}

fn scope(account: &str, app: &str, vault: &str, profile: &str) -> YandexCredentialScope {
    YandexCredentialScope::new(account, vault, profile, app).unwrap()
}

fn store() -> YandexCredentialVault {
    YandexCredentialVault { backend: Box::<MemoryBackend>::default() }
}

#[test]
fn keyring_scope_requires_verified_account_app_vault_and_profile() {
    let own = scope("1000034426", APP, VAULT, "default");
    assert!(own.is_same_scope(&scope("1000034426", APP, VAULT, "default")));
    assert!(!own.is_same_scope(&scope("1000034427", APP, VAULT, "default")));
    assert!(!own.is_same_scope(&scope("1000034426", "different_app", VAULT, "default")));
    assert!(!own.is_same_scope(&scope("1000034426", APP, "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "default")));
    assert!(!own.is_same_scope(&scope("1000034426", APP, VAULT, "other")));
    for label in ["1000034426", APP, VAULT, "default"] {
        assert!(!own.username().contains(label));
        assert!(!format!("{own:?}").contains(label));
    }
    assert_eq!(own.username().len(), 67);
}

#[test]
fn invalid_identity_scope_and_token_are_rejected() {
    let wrong = [
        ("alice", APP, VAULT, "default"),
        ("", APP, VAULT, "default"),
        ("1", "../evil", VAULT, "default"),
        ("1", APP, "invalid", "default"),
        ("1", APP, VAULT, "../escape"),
        ("1", APP, VAULT, ""),
    ];
    for (account, app, vault, profile) in wrong {
        assert!(matches!(
            YandexCredentialScope::new(account, vault, profile, app),
            Err(SecretError::InvalidScope)
        ));
    }
    for value in ["", "broken\nsecret", "not allowed"] {
        assert!(matches!(YandexRefreshToken::new(value.into()), Err(SecretError::InvalidToken)));
    }
}

#[test]
fn refresh_tokens_are_partitioned_rotated_and_forgotten_locally() {
    let vault = store();
    let alice = scope("1000034426", APP, VAULT, "default");
    let bob = scope("1000034427", APP, VAULT, "default");
    vault.save_refresh(&alice, &YandexRefreshToken::new("one-refresh".into()).unwrap()).unwrap();
    assert!(vault.load_refresh(&bob).unwrap().is_none());
    vault.save_refresh(&bob, &YandexRefreshToken::new("bob-refresh".into()).unwrap()).unwrap();
    vault.save_refresh(&alice, &YandexRefreshToken::new("two-refresh".into()).unwrap()).unwrap();
    assert_eq!(vault.load_refresh(&alice).unwrap().unwrap().as_str(), "two-refresh");
    assert_eq!(vault.inspect_local(&alice).unwrap(), StoredRefreshPresence::PresentUnverified);
    vault.forget_local(&alice).unwrap();
    assert_eq!(vault.inspect_local(&alice).unwrap(), StoredRefreshPresence::Missing);
    assert!(vault.load_refresh(&alice).unwrap().is_none());
    assert_eq!(vault.load_refresh(&bob).unwrap().unwrap().as_str(), "bob-refresh");
}

#[test]
fn mismatch_errors_and_debug_never_expose_token_contents() {
    let s = scope("1000034426", APP, VAULT, "default");
    let token = YandexRefreshToken::new("secret-refresh".into()).unwrap();
    assert_eq!(format!("{token:?}"), "YandexRefreshToken(REDACTED)");
    assert_eq!(format!("{:?}", store()), "YandexCredentialVault(REDACTED)");
    let v = YandexCredentialVault { backend: Box::new(MemoryBackend {
        fail: true, ..Default::default()
    }) };
    assert_eq!(v.save_refresh(&s, &token), Err(SecretError::StoreFailure));
    assert!(matches!(v.load_refresh(&s), Err(SecretError::StoreFailure)));
    let v = YandexCredentialVault { backend: Box::new(MemoryBackend {
        corrupt: true, ..Default::default()
    }) };
    assert_eq!(v.save_refresh(&s, &token), Err(SecretError::VerificationFailed));
    assert!(!format!("{token:?}").contains("secret-refresh"));
}

#[test]
fn disabled_native_feature_has_no_insecure_fallback() {
    if !cfg!(feature = "system-store") {
        assert!(matches!(YandexCredentialVault::system(), Err(SecretError::StoreUnavailable)));
    }
    assert_eq!(SERVICE, "org.akula.yandex-disk.app-folder.v1");
}

#[test]
fn local_token_presence_never_means_yandex_authorization() {
    let vault = store();
    let alice = scope("1000034426", APP, VAULT, "default");
    let bob = scope("1000034427", APP, VAULT, "default");
    assert_eq!(vault.inspect_local(&alice), Ok(StoredRefreshPresence::Missing));
    vault.backend.put(alice.username(), "unverified-refresh").unwrap();
    assert_eq!(vault.inspect_local(&alice), Ok(StoredRefreshPresence::PresentUnverified));
    assert_eq!(vault.inspect_local(&bob), Ok(StoredRefreshPresence::Missing));
    vault.backend.put(alice.username(), "invalid refresh with spaces").unwrap();
    assert_eq!(vault.inspect_local(&alice), Err(SecretError::InvalidToken));
}

struct DishonestDelete {
    keeps_credential: bool,
    unreadable: bool,
}

impl Backend for DishonestDelete {
    fn put(&self, _: &str, _: &str) -> Result<(), SecretError> { Ok(()) }
    fn get(&self, _: &str) -> Result<Option<Zeroizing<String>>, SecretError> {
        if self.unreadable { return Err(SecretError::StoreFailure); }
        Ok(self.keeps_credential.then(|| Zeroizing::new("secret-remains".to_owned())))
    }
    fn delete(&self, _: &str) -> Result<(), SecretError> { Ok(()) }
}

#[test]
fn successful_delete_without_verified_absence_is_rejected() {
    let alice = scope("1000034426", APP, VAULT, "default");
    let stale = YandexCredentialVault {
        backend: Box::new(DishonestDelete { keeps_credential: true, unreadable: false }),
    };
    assert_eq!(stale.forget_local(&alice), Err(SecretError::VerificationFailed));
    let inaccessible = YandexCredentialVault {
        backend: Box::new(DishonestDelete { keeps_credential: false, unreadable: true }),
    };
    assert_eq!(inaccessible.forget_local(&alice), Err(SecretError::StoreFailure));
    let absent = YandexCredentialVault {
        backend: Box::new(DishonestDelete { keeps_credential: false, unreadable: false }),
    };
    assert_eq!(absent.forget_local(&alice), Ok(()));
}

#[test]
fn presence_and_errors_do_not_reveal_tokens() {
    let token = YandexRefreshToken::new("very-secret-refresh-token".into()).unwrap();
    assert_eq!(format!("{:?}", StoredRefreshPresence::PresentUnverified), "PresentUnverified");
    assert_eq!(format!("{:?}", SecretError::VerificationFailed), "VerificationFailed");
    assert!(!format!("{token:?}").contains("very-secret-refresh-token"));
}
