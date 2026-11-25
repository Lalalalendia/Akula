use super::*;
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

#[test]
fn existing_yandex_keyring_username_format_is_stable() {
    // Golden value independently computed with SHA-256 over the existing
    // domain separator and little-endian length-prefixed synthetic labels.
    let own = scope("1000034426", "fixture_app", VAULT, "default");
    assert_eq!(own.username(), "v1_828d1ea8b486a82715ad1a5d79b76f3c89f758b44cbf7af0bd7e94f20198e00c");
}

struct FixedReadback {
    expected_scope: String,
    put_result: Result<(), SecretError>,
    read_result: Result<Option<&'static str>, SecretError>,
    wrote: std::sync::atomic::AtomicBool,
}

impl Backend for FixedReadback {
    fn put(&self, username: &str, token: &str) -> Result<(), SecretError> {
        assert_eq!(username, self.expected_scope);
        assert_eq!(token, "fixture-token");
        assert!(!self.wrote.swap(true, std::sync::atomic::Ordering::SeqCst));
        self.put_result
    }
    fn get(&self, username: &str) -> Result<Option<Zeroizing<String>>, SecretError> {
        assert_eq!(username, self.expected_scope);
        assert!(self.put_result.is_ok(), "failed write must not trigger readback");
        assert!(self.wrote.load(std::sync::atomic::Ordering::SeqCst));
        self.read_result.map(|v| v.map(|s| Zeroizing::new(s.to_owned())))
    }
    fn delete(&self, _: &str) -> Result<(), SecretError> {
        panic!("save must not delete an existing credential")
    }
}

#[test]
fn save_requires_exact_readback_and_preserves_os_errors() {
    let scope = scope("1000034426", "fixture_app", VAULT, "default");
    let token = YandexRefreshToken::new("fixture-token".into()).unwrap();
    for (put, read, expected) in [
        (Ok(()), Ok(Some("fixture-token")), Ok(())),
        (Ok(()), Ok(Some("fixture-tokem")), Err(SecretError::VerificationFailed)),
        (Ok(()), Ok(Some("short")), Err(SecretError::VerificationFailed)),
        (Ok(()), Ok(None), Err(SecretError::VerificationFailed)),
        (Ok(()), Err(SecretError::StoreFailure), Err(SecretError::StoreFailure)),
        (Err(SecretError::StoreUnavailable), Ok(Some("fixture-token")), Err(SecretError::StoreUnavailable)),
    ] {
        let vault = YandexCredentialVault {
            backend: Box::new(FixedReadback {
                expected_scope: scope.username().to_owned(),
                put_result: put,
                read_result: read,
                wrote: std::sync::atomic::AtomicBool::new(false),
            }),
        };
        assert_eq!(vault.save_refresh(&scope, &token), expected);
    }
}

#[test]
fn stored_token_validation_preserves_size_and_ascii_boundaries() {
    let scope = scope("1000034426", "fixture_app", VAULT, "default");
    let vault = store();
    let max = "x".repeat(4096);
    let token = YandexRefreshToken::new(max.clone()).unwrap();
    vault.save_refresh(&scope, &token).unwrap();
    assert_eq!(vault.load_refresh(&scope).unwrap().unwrap().as_str(), max.as_str());
    for invalid in ["".to_owned(), "x".repeat(4097), "with space".to_owned(),
                    "bad\nvalue".to_owned(), "токен".to_owned()] {
        assert!(matches!(YandexRefreshToken::new(invalid.clone()), Err(SecretError::InvalidToken)));
        vault.backend.put(scope.username(), &invalid).unwrap();
        assert!(matches!(vault.load_refresh(&scope), Err(SecretError::InvalidToken)));
    }
}
