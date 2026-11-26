//! Opt-in native personal Yandex Disk read-only onboarding preview.
//! Only explicit Settings button actions can launch a system browser.
//! No Sync Core grants, no note writes, no S3 credentials, no automatic startup.
//!
//! The OAuth product-owned public client_id/registered callback port are baked
//! into *non-installed* QA builds. Missing values disable the login button.
use akula_yandex_disk_connect_lab::{ConnectError, ConnectSession, ReadOnlyReceipt, ReadOnlyReview};
use std::{
    path::Path,
    sync::{mpsc::{self, Receiver, Sender}, Arc},
    time::Duration,
};
const APP_ID: Option<&str> = option_env!("AKULA_YANDEX_OAUTH_CLIENT_ID");
const PORT_TEXT: Option<&str> = option_env!("AKULA_YANDEX_OAUTH_CALLBACK_PORT");
const PROFILE: &str = "personal_disk";
fn registered_identity() -> Option<(&'static str, u16)> {
    let client = APP_ID?;
    let port = PORT_TEXT?.parse::<u16>().ok()?;
    if port < 1024 || client.len() < 4 || client.len() > 128
        || !client.bytes().all(|v| v.is_ascii_alphanumeric() || v == b'-' || v == b'_')
    {
        return None;
    }
    Some((client, port))
}
#[path = "oauth_browser.rs"]
mod browser;
use browser::open_browser;
#[path = "yandex_disk_preview/lifecycle.rs"] mod lifecycle;
enum Reply {
    Reviewed(Result<ReadOnlyReview, ConnectError>),
    Saved(Result<ReadOnlyReceipt, ConnectError>),
    LocalInspected(Result<akula_yandex_disk_connect_lab::local_lifecycle::StoredRefreshPresence, ConnectError>),
    LocalForgot(Result<aquilum_core::settings::models::YandexDiskPublicBinding, ConnectError>),
    BrowserUnavailable,
    WorkerFailed,
}
pub(super) struct YandexDiskPreview {
    pub(super) status: &'static str,
    pub(super) busy: bool,
    pub(super) review_account: Option<String>,
    review: Option<ReadOnlyReview>,
    pending_forget: Option<aquilum_core::settings::models::YandexDiskPublicBinding>,
    vault: Option<String>,
    generation: u64,
    tx: Sender<(u64, Reply)>,
    rx: Receiver<(u64, Reply)>,
}
impl YandexDiskPreview {
    pub(super) fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            status: "YANDEX_PERSONAL_DISK_DISCONNECTED / SYNC_DISABLED",
            busy: false,
            review_account: None,
            review: None,
            pending_forget: None,
            vault: None,
            generation: 0,
            tx,
            rx,
        }
    }
    pub(super) fn has_registered_app() -> bool {
        registered_identity().is_some()
    }
    /// Return a public status only. ReadOnlyReview itself contains a refresh
    /// token and MUST NOT be stored in AppConfig, traced or formatted.
    pub(super) fn start(&mut self, vault_id: Option<String>, wake: Arc<dyn Fn() + Send + Sync>) {
        if self.busy {
            return;
        }
        let Some((app, port)) = registered_identity() else {
            self.status = "YANDEX_OAUTH_APP_NOT_REGISTERED / SYNC_DISABLED";
            return;
        };
        let Some(vault) = vault_id else {
            self.status = "YANDEX_LOCAL_VAULT_NOT_SELECTED / SYNC_DISABLED";
            return;
        };
        self.review = None;
        self.review_account = None;
        self.vault = Some(vault.clone());
        self.generation = self.generation.wrapping_add(1);
        let generation_token = self.generation;
        self.busy = true;
        self.status = "YANDEX_WAITING_FOR_BROWSER_CONSENT / SYNC_DISABLED";
        let tx = self.tx.clone();
        let spawn = std::thread::Builder::new()
            .name("akula-yandex-disk-login".into())
            .spawn(move || {
                // Catch a panicking transport worker without logging tokens.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let session = ConnectSession::begin(app, port, &vault, PROFILE);
                    match session {
                        Ok(session) if open_browser(session.browser_authorize_url()) =>
                            Reply::Reviewed(session.finish(Duration::from_secs(120))),
                        Ok(_) => Reply::BrowserUnavailable,
                        Err(err) => Reply::Reviewed(Err(err)),
                    }
                })).unwrap_or(Reply::WorkerFailed);
                if tx.send((generation_token, outcome)).is_ok() {
                    wake();
                }
            });
        if spawn.is_err() {
            self.busy = false;
            self.status = "YANDEX_WORKER_UNAVAILABLE / SYNC_DISABLED";
        }
    }
    /// Explicit second click. The button label displays the same account ID;
    /// never auto-save on callback or page rendering. Keyring IO stays on
    /// its own OS worker, with no plaintext file fallback.
    pub(super) fn confirm(&mut self, current_vault: Option<&str>,
        wake: Arc<dyn Fn() + Send + Sync>)
    {
        if self.busy || self.vault.as_deref() != current_vault {
            self.discard_if_workspace_changed(current_vault);
            return;
        }
        let Some(review) = self.review.take() else {
            return;
        };
        let Some(account) = self.review_account.take() else {
            self.status = "YANDEX_REVIEW_NO_ACCOUNT / SYNC_DISABLED";
            return;
        };
        self.busy = true;
        self.status = "YANDEX_STORING_OS_CREDENTIAL / SYNC_DISABLED";
        let generation_token = self.generation;
        let tx = self.tx.clone();
        let spawned = std::thread::Builder::new()
            .name("akula-yandex-disk-os-keyring".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    Reply::Saved(review.confirm_and_save(&account))
                })).unwrap_or(Reply::WorkerFailed);
                if tx.send((generation_token, outcome)).is_ok() {
                    wake();
                }
            });
        if spawned.is_err() {
            self.busy = false;
            self.status = "YANDEX_WORKER_UNAVAILABLE / SYNC_DISABLED";
        }
    }
    fn discard_if_workspace_changed(&mut self, current_vault: Option<&str>) -> bool {
        if self.vault.is_some() && self.vault.as_deref() != current_vault {
            self.generation = self.generation.wrapping_add(1);
            self.review = None; self.review_account = None; self.pending_forget = None;
            self.status = "YANDEX_WORKSPACE_CHANGED / SYNC_DISABLED";
            // Preserve busy until the old worker finishes, to avoid opening
            // two browser listeners or discarding a pending OS keyring write.
            return true;
        }
        false
    }
    pub(super) fn poll(&mut self, current_vault: Option<&str>) -> bool {
        let mut changed = self.discard_if_workspace_changed(current_vault);
        while let Ok((generation_token, reply)) = self.rx.try_recv() {
            // A stale callback cannot grant a review for another vault.
            if generation_token != self.generation {
                self.busy = false;
                changed = true;
                continue;
            }
            self.busy = false;
            changed = true;
            match reply {
                Reply::Reviewed(Ok(review)) => {
                    self.review_account = Some(review.observed_yandex_account_id().to_owned());
                    self.review = Some(review);
                    self.status = "YANDEX_REVIEW_ACCOUNT_BEFORE_SAVING / SYNC_DISABLED";
                }
                Reply::Reviewed(Err(err)) => {
                    self.status = err.public_status();
                }
                Reply::Saved(Ok(receipt)) => {
                    debug_assert!(!receipt.is_sync_enabled());
                    self.status = "YANDEX_READONLY_CONSENT_SAVED / SYNC_DISABLED";
                }
                Reply::Saved(Err(err)) => self.status = err.public_status(),
                Reply::LocalInspected(Ok(akula_yandex_disk_connect_lab::local_lifecycle::StoredRefreshPresence::Missing)) => self.status = "YANDEX_LOCAL_TOKEN_MISSING / SYNC_DISABLED",
                Reply::LocalInspected(Ok(akula_yandex_disk_connect_lab::local_lifecycle::StoredRefreshPresence::PresentUnverified)) => self.status = "YANDEX_LOCAL_TOKEN_PRESENT_UNVERIFIED / SYNC_DISABLED",
                Reply::LocalInspected(Err(err)) => self.status = err.public_status(),
                Reply::LocalForgot(Ok(record)) => { self.pending_forget = Some(record); self.status = "YANDEX_LOCAL_FORGET_VERIFYING_SETTINGS / SYNC_DISABLED"; }
                Reply::LocalForgot(Err(err)) => self.status = err.public_status(),
                Reply::BrowserUnavailable => {
                    self.status = "YANDEX_BROWSER_UNAVAILABLE / SYNC_DISABLED";
                }
                Reply::WorkerFailed => {
                    self.status = "YANDEX_WORKER_FAILED / SYNC_DISABLED";
                }
            }
        }
        changed
    }
}
impl super::App {
    /// Device-local UiState UUID, NEVER a provider's cloud Vault identity.
    /// Do not create a new identity or silently pair another workspace.
    pub(super) fn yandex_disk_local_vault(&self) -> Option<String> {
        let selected = self.tree.root()?;
        let selected = crate::workspace::normalize(selected);
        self.workspace.core.ui_state.list_workspaces().ok()?
            .into_iter()
            .find(|entry| crate::workspace::normalize(Path::new(&entry.path)) == selected)
            .map(|entry| entry.id.to_string())
    }
    pub(super) fn press_yandex_disk(&mut self,
        action: crate::ui::settings::schema::Action,
        section: crate::ui::settings::schema::Section,
    ) -> super::settings::Effect {
        use crate::ui::settings::schema::{Action, Section};
        if section != Section::Sync {
            return super::settings::Effect::None;
        }
        let vault = self.yandex_disk_local_vault();
        let wake = Arc::clone(&self.workspace.wake);
        match action {
            Action::YandexDiskLoginReadOnly => {
                self.yandex_disk_preview.start(vault, wake);
            }
            Action::YandexDiskConfirmAccount => {
                if self.remember_yandex_public_review(vault.as_deref()) {
                    self.yandex_disk_preview.confirm(vault.as_deref(), wake);
                }
            }
            Action::YandexDiskInspectLocal => { let record = self.yandex_local_public_hint(vault.as_deref()); self.yandex_disk_preview.start_local_inspect(record, wake); }
            Action::YandexDiskForgetLocal => { let record = self.yandex_local_public_hint(vault.as_deref()); self.yandex_disk_preview.start_local_forget(record, wake); }
            _ => {}
        }
        super::settings::Effect::Content
    }

    pub(super) fn poll_yandex_disk_settings(
        &mut self,
        root: &mut masonry::app::RenderRoot,
    ) {
        let vault = self.yandex_disk_local_vault();
        let changed = self.yandex_disk_preview.poll(vault.as_deref());
        let changed = self.reconcile_yandex_local_forget() || changed;
        if changed && matches!(&self.popup,
            Some(super::Popup::Settings { dialog, .. })
                if dialog.section == crate::ui::settings::schema::Section::Sync)
        {
            self.refresh_settings(root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_compiled_product_registration_never_starts_a_login() {
        if registered_identity().is_none() {
            let mut preview = YandexDiskPreview::new();
            preview.start(
                Some("12345678-1234-1234-1234-123456789abc".into()),
                Arc::new(|| {}),
            );
            assert_eq!(preview.status,
                "YANDEX_OAUTH_APP_NOT_REGISTERED / SYNC_DISABLED");
            assert!(!preview.busy);
        }
    }

    #[test]
    fn changing_workspace_discards_pending_review_and_stale_worker() {
        let mut preview = YandexDiskPreview::new();
        preview.vault = Some("vault-a".into());
        preview.busy = true;
        let generation = preview.generation;
        assert!(preview.poll(Some("vault-b")));
        assert!(preview.busy);
        assert_eq!(preview.status, "YANDEX_WORKSPACE_CHANGED / SYNC_DISABLED");
        preview.tx.send((generation, Reply::WorkerFailed)).unwrap();
        assert!(preview.poll(Some("vault-b")));
        assert!(!preview.busy);
        assert!(preview.review_account.is_none());
        assert_eq!(preview.status, "YANDEX_WORKSPACE_CHANGED / SYNC_DISABLED");
    }

    #[test]
    fn confirmation_without_review_is_inert() {
        let mut preview = YandexDiskPreview::new();
        preview.vault = Some("vault-a".into());
        preview.confirm(Some("vault-a"), Arc::new(|| {}));
        assert!(!preview.busy);
        assert!(preview.review_account.is_none());
        assert!(preview.review.is_none());
    }
}
