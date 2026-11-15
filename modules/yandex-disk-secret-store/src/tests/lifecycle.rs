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
