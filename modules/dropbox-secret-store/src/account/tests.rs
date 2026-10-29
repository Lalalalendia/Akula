use super::*;

#[test]
fn account_identity_is_separate_from_vault_and_other_apps() {
    let a = AccountCredentialScope::new("akula_app", "dbid:alice", "12345").unwrap();
    // Account keys must remain byte-for-byte identical after SHA-2 upgrades.
    assert_eq!(a.username,
        "account_b3fcf4db47cdca36b5fb99753ff49873fe031ef1f6580e15562cfe56b3d5a9a0");
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
