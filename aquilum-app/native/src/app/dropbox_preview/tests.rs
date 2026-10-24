use super::*;

#[test]
fn missing_scope_does_not_start_browser_or_network() {
    let mut state = DropboxPreview::new();
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
    state.start(DropboxReadOnlyDraft::default(), false, wake);
    assert!(!state.busy);
    assert_eq!(state.status, "INVALID_TEST_CONFIG / SYNC_DISABLED");
}

#[test]
fn config_change_rejects_late_success_from_previous_account() {
    let mut state = DropboxPreview::new();
    let original_generation = state.generation;
    state.invalidate();
    assert_ne!(state.generation, original_generation);
    assert!(!state.busy);
    let _ = state.tx.send((
        original_generation,
        PreviewReply::Vault("REMOTE_PINNED_READ_OK / PUBLISHER_NOT_APPROVED / SYNC_DISABLED"),
    ));
    assert_eq!(state.poll(), (false, None, None));
    assert_eq!(state.status, "DISCONNECTED / SYNC_DISABLED");
}

#[test]
fn account_login_has_no_vault_prerequisite_and_cannot_enable_sync() {
    let draft = DropboxReadOnlyDraft::default();
    assert!(!draft.ready());
    assert_eq!(draft.oauth_app_key, AKULA_DROPBOX_APP_KEY);
    assert_eq!(draft.registered_port, AKULA_DROPBOX_CALLBACK_PORT);
    let mut state = DropboxPreview::new();
    let valid = AccountIdentity {
        account_id: "dbid:alice".into(),
        root_namespace_id: "12345".into(),
    };
    state.tx.send((state.generation,
        PreviewReply::Account(Ok(valid.clone())))).unwrap();
    assert_eq!(state.poll(), (true, Some(valid), None));
    assert!(state.status.contains("SYNC_DISABLED"));
}

#[test]
fn changed_identity_discards_late_account_consent() {
    let mut state = DropboxPreview::new();
    let generation = state.generation;
    state.invalidate();
    state.tx.send((generation, PreviewReply::Account(Ok(AccountIdentity {
        account_id: "dbid:old".into(),
        root_namespace_id: "1".into(),
    })))).unwrap();
    assert_eq!(state.poll(), (false, None, None));
    assert!(state.status.contains("DISCONNECTED"));
}

#[test]
fn local_forget_only_returns_the_exact_account_identity() {
    let mut state = DropboxPreview::new();
    let account = AccountIdentity {
        account_id: "dbid:alice".into(), root_namespace_id: "12345".into(),
    };
    state.tx.send((state.generation,
        PreviewReply::SignedOut(Ok(account.clone())))).unwrap();
    assert_eq!(state.poll(), (true, None, Some(account)));
    assert_eq!(state.status, "LOCAL_CREDENTIAL_FORGOTTEN / SYNC_DISABLED");
}

#[test]
fn failed_os_delete_never_claims_to_have_disconnected() {
    let mut state = DropboxPreview::new();
    state.tx.send((state.generation, PreviewReply::SignedOut(Err(())))).unwrap();
    assert_eq!(state.poll(), (true, None, None));
    assert_eq!(state.status, "LOCAL_CREDENTIAL_REMOVAL_FAILED / SYNC_DISABLED");
}

#[test]
fn cold_restart_resume_requires_saved_public_account_only() {
    let mut state = DropboxPreview::new();
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
    state.start_account_resume(DropboxReadOnlyDraft::default(), wake);
    assert!(!state.busy);
    assert_eq!(state.status, "ACCOUNT_RESUME_INVALID_IDENTITY / SYNC_DISABLED");
    let identity = AccountIdentity {
        account_id: "dbid:alice".into(),
        root_namespace_id: "12345".into(),
    };
    state.tx.send((state.generation,
        PreviewReply::Resumed(Ok(identity.clone())))).unwrap();
    assert_eq!(state.poll(), (true, Some(identity), None));
    assert_eq!(state.status,
        "ACCOUNT_SESSION_RESTORED / VAULT_NOT_APPROVED / SYNC_DISABLED");
}

#[test]
fn stale_or_failed_refresh_never_recovers_authority() {
    let mut state = DropboxPreview::new();
    let old_generation = state.generation;
    state.invalidate();
    state.tx.send((old_generation, PreviewReply::Resumed(Ok(AccountIdentity {
        account_id: "dbid:old".into(), root_namespace_id: "1".into(),
    })))).unwrap();
    assert_eq!(state.poll(), (false, None, None));
    state.tx.send((state.generation, PreviewReply::Resumed(Err(AccountLoginError::MissingSavedConsent)))).unwrap();
    assert_eq!(state.poll(), (true, None, None));
    assert_eq!(state.status, "DROPBOX_NO_SAVED_CONSENT / SYNC_DISABLED");
}

#[test]
fn browser_port_and_revoke_failures_display_distinct_safe_categories() {
    let mut state = DropboxPreview::new();
    state.tx.send((state.generation, PreviewReply::Account(
        Err(AccountLoginError::BrowserUnavailable)))).unwrap();
    assert_eq!(state.poll(), (true, None, None));
    assert_eq!(state.status, "DROPBOX_BROWSER_UNAVAILABLE / SYNC_DISABLED");
    state.tx.send((state.generation, PreviewReply::Resumed(
        Err(AccountLoginError::Refresh(
            akula_dropbox_connect_lab::resume::ResumeError::RefreshRejected,
        ))))).unwrap();
    assert_eq!(state.poll(), (true, None, None));
    assert_eq!(state.status, "DROPBOX_SAVED_CONSENT_REVOKED / SYNC_DISABLED");
    assert!(!state.status.contains("Bearer"));
}

#[test]
fn preview_never_reports_sync_enabled() {
    let state = DropboxPreview::new();
    assert!(state.status.contains("SYNC_DISABLED"));
}

#[test]
fn repeat_oauth_consent_is_bound_to_exact_saved_public_identity() {
    let mut draft = DropboxReadOnlyDraft::default();
    assert_eq!(existing_account_scope(draft.clone()), None);
    draft.account_id = "dbid:alice".into();
    draft.root_namespace_id = "12345".into();
    assert_eq!(
        existing_account_scope(draft.clone()),
        Some(AccountIdentity {
            account_id: "dbid:alice".into(),
            root_namespace_id: "12345".into(),
        }),
    );
    draft.root_namespace_id.clear();
    assert_eq!(existing_account_scope(draft), None);
}
