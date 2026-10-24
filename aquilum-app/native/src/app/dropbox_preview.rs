//! Akula 3.13: explicit, read-only Dropbox test connection from native UI.
//! Never mutates local files, publishes remote data or changes Sync Core state.
#[path = "dropbox_provider.rs"]
mod provider;

use aquilum_core::settings::models::{
    DropboxReadOnlyDraft, AKULA_DROPBOX_APP_KEY, AKULA_DROPBOX_CALLBACK_PORT,
};
use akula_dropbox_connect_lab::{
    account::{resume_account, AccountIdentity, AccountLoginError, AccountSession},
    resume::resume_readonly, ConnectSession, ConnectionStatus,
};
use akula_dropbox_t0::{read_verify::TrustedBlake3, Scope};
use akula_dropbox_secret_store::{
    account::{AccountCredentialScope, AccountCredentialStore},
    CredentialScope, CredentialVault,
};
use std::{
    process::{Command, Stdio},
    sync::{mpsc::{channel, Receiver, Sender}, Arc},
    time::Duration,
};

// Only the account proof may update the saved public account/root identifiers.
// A Vault result is never converted to write/sync authority.
enum PreviewReply {
    Vault(&'static str),
    Account(Result<AccountIdentity, AccountLoginError>),
    Resumed(Result<AccountIdentity, AccountLoginError>),
    SignedOut(Result<AccountIdentity, ()>),
}

pub(super) struct DropboxPreview {
    pub(super) status: String,
    pub(super) busy: bool,
    generation: u64,
    tx: Sender<(u64, PreviewReply)>,
    rx: Receiver<(u64, PreviewReply)>,
}
impl DropboxPreview {
    pub(super) fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            status: "DISCONNECTED / SYNC_DISABLED".to_owned(),
            busy: false,
            generation: 0,
            tx,
            rx,
        }
    }

    /// Account-only OAuth is independent of VaultId/profile/HEAD and grants
    /// neither a confirmed baseline nor permission to apply remote documents.
    pub(super) fn start_account(
        &mut self, draft: DropboxReadOnlyDraft,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        if self.busy { return; }
        if draft.oauth_app_key != AKULA_DROPBOX_APP_KEY
            || draft.registered_port != AKULA_DROPBOX_CALLBACK_PORT
        {
            self.status = "INVALID_AKULA_OAUTH_IDENTITY / SYNC_DISABLED".into();
            return;
        }
        self.busy = true;
        self.status = "WAITING_FOR_DROPBOX_ACCOUNT_CONSENT / SYNC_DISABLED".into();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let tx = self.tx.clone();
        let _ = std::thread::Builder::new()
            .name("akula-dropbox-account".into())
            .spawn(move || {
                // Bind loopback before launching the system browser. Never
                // print the OAuth URL, authorization code or access token.
                let result = AccountSession::begin(
                    AKULA_DROPBOX_APP_KEY, AKULA_DROPBOX_CALLBACK_PORT,
                ).and_then(|session| {
                    if !open_browser(session.browser_authorize_url()) {
                        return Err(AccountLoginError::BrowserUnavailable);
                    }
                    session.finish(Duration::from_secs(120))
                });
                if tx.send((generation, PreviewReply::Account(result))).is_ok() {
                    wake();
                }
            })
            .map_err(|_| {
                self.busy = false;
                self.status = "THREAD_START_FAILED / SYNC_DISABLED".into();
            });
    }

    /// Manual cold-start recovery from the OS secret store. This never
    /// opens a browser or treats an OAuth identity as a trusted Vault.
    pub(super) fn start_account_resume(
        &mut self, draft: DropboxReadOnlyDraft,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        if self.busy { return; }
        if draft.oauth_app_key != AKULA_DROPBOX_APP_KEY
            || draft.registered_port != AKULA_DROPBOX_CALLBACK_PORT
            || draft.account_id.is_empty() || draft.root_namespace_id.is_empty()
        {
            self.status = "ACCOUNT_RESUME_INVALID_IDENTITY / SYNC_DISABLED".into();
            return;
        }
        self.busy = true;
        self.status = "VERIFYING_SAVED_DROPBOX_CONSENT / SYNC_DISABLED".into();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let tx = self.tx.clone();
        let _ = std::thread::Builder::new()
            .name("akula-dropbox-account-resume".into())
            .spawn(move || {
                let expected = AccountIdentity {
                    account_id: draft.account_id,
                    root_namespace_id: draft.root_namespace_id,
                };
                let result = resume_account(AKULA_DROPBOX_APP_KEY, &expected);
                if tx.send((generation, PreviewReply::Resumed(result))).is_ok() {
                    wake();
                }
            })
            .map_err(|_| {
                self.busy = false;
                self.status = "THREAD_START_FAILED / SYNC_DISABLED".into();
            });
    }

    /// Forget only this device's approved account and optional Vault tokens.
    /// Do not claim remote revocation, and do not clear settings on OS failure.
    pub(super) fn start_logout(
        &mut self, draft: DropboxReadOnlyDraft,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        if self.busy { return; }
        if draft.oauth_app_key != AKULA_DROPBOX_APP_KEY
            || draft.registered_port != AKULA_DROPBOX_CALLBACK_PORT
            || draft.account_id.is_empty() || draft.root_namespace_id.is_empty()
        {
            self.status = "ACCOUNT_LOGOUT_INVALID_IDENTITY / SYNC_DISABLED".into();
            return;
        }
        self.busy = true;
        self.status = "FORGETTING_LOCAL_DROPBOX_CREDENTIALS / SYNC_DISABLED".into();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let tx = self.tx.clone();
        let _ = std::thread::Builder::new()
            .name("akula-dropbox-account-forget".into())
            .spawn(move || {
                let result = (|| {
                    let scope = AccountCredentialScope::new(
                        &draft.oauth_app_key, &draft.account_id, &draft.root_namespace_id,
                    ).map_err(|_| ())?;
                    AccountCredentialStore::system().map_err(|_| ())?
                        .forget_local(&scope).map_err(|_| ())?;
                    // A paired Vault's refresh token lives in a separate
                    // credential namespace and must also be forgotten.
                    if draft.ready() {
                        let vault = CredentialScope::new(
                            &draft.account_id, &draft.root_namespace_id,
                            &draft.vault_id, &draft.profile_id, &draft.oauth_app_key,
                        ).map_err(|_| ())?;
                        CredentialVault::system().map_err(|_| ())?
                            .forget_local(&vault).map_err(|_| ())?;
                    }
                    Ok(AccountIdentity {
                        account_id: draft.account_id,
                        root_namespace_id: draft.root_namespace_id,
                    })
                })();
                if tx.send((generation, PreviewReply::SignedOut(result))).is_ok() {
                    wake();
                }
            })
            .map_err(|_| {
                self.busy = false;
                self.status = "THREAD_START_FAILED / SYNC_DISABLED".into();
            });
    }

    pub(super) fn start(
        &mut self,
        draft: DropboxReadOnlyDraft,
        resume: bool,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        if self.busy { return; }
        if !draft.ready() {
            self.status = "INVALID_TEST_CONFIG / SYNC_DISABLED".into();
            return;
        }
        // Test scope and pin are constructed before any browser or network action.
        let Ok(scope) = Scope::new(
            &draft.account_id, &draft.root_namespace_id,
            &draft.vault_id, &draft.profile_id,
        ) else {
            self.status = "INVALID_SCOPE / SYNC_DISABLED".into();
            return;
        };
        let Ok(pin) = TrustedBlake3::parse(&draft.trusted_head_blake3) else {
            self.status = "INVALID_HEAD_PIN / SYNC_DISABLED".into();
            return;
        };
        self.busy = true;
        self.status = if resume {
            "VERIFYING_STORED_READ_ONLY_CONNECTION"
        } else {
            "WAITING_FOR_DROPBOX_BROWSER_CONSENT"
        }.into();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let tx = self.tx.clone();
        // Never print the app key, OAuth URL, browser callback, token or HEAD.
        let _ = std::thread::Builder::new()
            .name("akula-dropbox-readonly".into())
            .spawn(move || {
                let outcome = if resume {
                    match resume_readonly(scope, &draft.oauth_app_key, pin) {
                        Ok(candidate) if candidate.status()
                            == ConnectionStatus::RemotePinnedNeedsPublisherApproval =>
                            "REMOTE_PINNED_READ_OK / PUBLISHER_NOT_APPROVED / SYNC_DISABLED",
                        _ => "READ_ONLY_RESUME_REJECTED / SYNC_DISABLED",
                    }
                } else {
                    match ConnectSession::begin(
                        scope, &draft.oauth_app_key, draft.registered_port, pin,
                    ) {
                        Ok(session) if open_browser(session.browser_authorize_url()) => {
                            match session.finish(Duration::from_secs(120)) {
                                Ok(candidate) if candidate.status()
                                    == ConnectionStatus::RemotePinnedNeedsPublisherApproval =>
                                    "REMOTE_PINNED_READ_OK / PUBLISHER_NOT_APPROVED / SYNC_DISABLED",
                                _ => "READ_ONLY_CONNECT_REJECTED / SYNC_DISABLED",
                            }
                        }
                        Ok(_) => "BROWSER_LAUNCH_FAILED / SYNC_DISABLED",
                        Err(_) => "LOGIN_BIND_REJECTED / SYNC_DISABLED",
                    }
                };
                if tx.send((generation, PreviewReply::Vault(outcome))).is_ok() { wake(); }
            })
            .map_err(|_| {
                self.busy = false;
                self.status = "THREAD_START_FAILED / SYNC_DISABLED".into();
            });
    }

    /// Editing account/root/head invalidates even an in-flight successful reply.
    /// A stale test must not be displayed as proof for newly selected identity.
    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.busy = false;
        self.status = "DISCONNECTED / SYNC_DISABLED".into();
    }

    pub(super) fn poll(&mut self)
        -> (bool, Option<AccountIdentity>, Option<AccountIdentity>)
    {
        let mut changed = false;
        let mut authenticated_account = None;
        let mut forgotten_account = None;
        while let Ok((generation, reply)) = self.rx.try_recv() {
            if generation != self.generation { continue; }
            self.busy = false;
            changed = true;
            match reply {
                PreviewReply::Vault(status) => self.status = status.into(),
                PreviewReply::Account(Ok(identity)) => {
                    self.status = "ACCOUNT_AUTHORIZED / VAULT_NOT_APPROVED / SYNC_DISABLED".into();
                    authenticated_account = Some(identity);
                }
                PreviewReply::Account(Err(err)) => {
                    self.status = err.public_status().into();
                }
                PreviewReply::Resumed(Ok(identity)) => {
                    self.status = "ACCOUNT_SESSION_RESTORED / VAULT_NOT_APPROVED / SYNC_DISABLED".into();
                    authenticated_account = Some(identity);
                }
                PreviewReply::Resumed(Err(err)) => {
                    self.status = err.public_status().into();
                }
                PreviewReply::SignedOut(Ok(identity)) => {
                    self.status = "LOCAL_CREDENTIAL_FORGOTTEN / SYNC_DISABLED".into();
                    forgotten_account = Some(identity);
                }
                PreviewReply::SignedOut(Err(_)) => {
                    self.status = "LOCAL_CREDENTIAL_REMOVAL_FAILED / SYNC_DISABLED".into();
                }
            }
        }
        (changed, authenticated_account, forgotten_account)
    }
}

// The current App/settings modules are intentionally kept within the
// repository source-size ratchet; Dropbox integration lives here.
impl super::App {
    pub(super) fn poll_dropbox_settings(
        &mut self, root: &mut masonry::app::RenderRoot,
    ) {
        let (changed, account, forgotten) = self.dropbox_preview.poll();
        if let Some(identity) = account {
            let mut config = self.workspace.core.settings.get_config();
            if config.dropbox_readonly.account_id != identity.account_id
                || config.dropbox_readonly.root_namespace_id != identity.root_namespace_id
            {
                // A different account invalidates all earlier Vault pairing.
                // No file, baseline, scheduler or sync state is modified.
                config.dropbox_readonly.vault_id.clear();
                config.dropbox_readonly.profile_id.clear();
                config.dropbox_readonly.trusted_head_blake3.clear();
                config.dropbox_readonly.account_id = identity.account_id;
                config.dropbox_readonly.root_namespace_id = identity.root_namespace_id;
                if self.workspace.core.settings.update_config(config).is_err() {
                    self.dropbox_preview.status =
                        "ACCOUNT_IDENTITY_SAVE_FAILED / SYNC_DISABLED".into();
                }
            }
        }
        if let Some(identity) = forgotten {
            let mut config = self.workspace.core.settings.get_config();
            if config.dropbox_readonly.account_id == identity.account_id
                && config.dropbox_readonly.root_namespace_id == identity.root_namespace_id
            {
                config.dropbox_readonly.account_id.clear();
                config.dropbox_readonly.root_namespace_id.clear();
                config.dropbox_readonly.vault_id.clear();
                config.dropbox_readonly.profile_id.clear();
                config.dropbox_readonly.trusted_head_blake3.clear();
                if self.workspace.core.settings.update_config(config).is_err() {
                    self.dropbox_preview.status =
                        "ACCOUNT_DISCONNECT_SAVE_FAILED / SYNC_DISABLED".into();
                }
            }
        }
        if changed
            && matches!(&self.popup, Some(super::Popup::Settings { dialog, .. })
                if dialog.section == crate::ui::settings::schema::Section::Sync)
        {
            self.refresh_settings(root);
        }
    }

    pub(super) fn press_dropbox(
        &mut self, action: crate::ui::settings::schema::Action,
        section: crate::ui::settings::schema::Section,
    ) -> super::settings::Effect {
        use crate::ui::settings::schema::{Action, Section};
        if section != Section::Sync {
            return super::settings::Effect::None;
        }
        let config = self.workspace.core.settings.get_config().dropbox_readonly;
        let wake = std::sync::Arc::clone(&self.workspace.wake);
        if matches!(&action, Action::DropboxLoginAccount) {
            self.dropbox_preview.start_account(config, wake);
        } else if matches!(&action, Action::DropboxForgetAccount) {
            self.dropbox_preview.start_logout(config, wake);
        } else if matches!(&action, Action::DropboxResumeAccount) {
            self.dropbox_preview.start_account_resume(config, wake);
        } else {
            let resume = matches!(action, Action::DropboxResumeReadOnly);
            self.dropbox_preview.start(config, resume, wake);
        }
        super::settings::Effect::Content
    }
}

fn open_browser(url: &str) -> bool {
    let binary = if cfg!(target_os = "windows") {
        "explorer.exe"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    // Argument is passed directly, without shell interpretation. OAuth URL
    // and children stdout/stderr are never included in application logs.
    Command::new(binary).arg(url)
        .stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().is_ok()
}

#[cfg(test)]
mod tests {
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
}
