use super::*;

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
    let scope = scope("dbid:alice", VAULT);
    let token = RefreshToken::new("fixture-token".into()).unwrap();
    for (put, read, expected) in [
        (Ok(()), Ok(Some("fixture-token")), Ok(())),
        (Ok(()), Ok(Some("fixture-tokem")), Err(SecretError::VerificationFailed)),
        (Ok(()), Ok(Some("short")), Err(SecretError::VerificationFailed)),
        (Ok(()), Ok(None), Err(SecretError::VerificationFailed)),
        (Ok(()), Err(SecretError::StoreFailure), Err(SecretError::StoreFailure)),
        (Err(SecretError::StoreUnavailable), Ok(Some("fixture-token")), Err(SecretError::StoreUnavailable)),
    ] {
        let vault = CredentialVault {
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
    let scope = scope("dbid:alice", VAULT);
    let vault = test_vault();
    let max = "x".repeat(4096);
    let token = RefreshToken::new(max.clone()).unwrap();
    vault.save_refresh(&scope, &token).unwrap();
    assert_eq!(vault.load_refresh(&scope).unwrap().unwrap().as_str(), max.as_str());
    for invalid in ["".to_owned(), "x".repeat(4097), "with space".to_owned(),
                    "bad\nvalue".to_owned(), "токен".to_owned()] {
        assert!(matches!(RefreshToken::new(invalid.clone()), Err(SecretError::InvalidToken)));
        vault.backend.put(scope.username(), &invalid).unwrap();
        assert!(matches!(vault.load_refresh(&scope), Err(SecretError::InvalidToken)));
    }
}
