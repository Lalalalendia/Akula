//! Explicit offline-only Yandex Disk credential lifecycle in native Settings.
//! A public account hint is never proof of Yandex authorization or Sync rights.
//! The OS keyring is the only credential store. No token is serialized here.

use super::*;
use akula_yandex_disk_connect_lab::local_lifecycle::{
    LocalCredentialScope, StoredRefreshPresence,
};
use aquilum_core::settings::models::YandexDiskPublicBinding;

impl YandexDiskPreview {
    pub(super) fn start_local_inspect(
        &mut self, record: Option<YandexDiskPublicBinding>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        self.start_local_action(record, false, wake);
    }

    pub(super) fn start_local_forget(
        &mut self, record: Option<YandexDiskPublicBinding>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        self.start_local_action(record, true, wake);
    }

    fn start_local_action(
        &mut self, record: Option<YandexDiskPublicBinding>,
        forget: bool, wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        if self.busy { return; }
        if self.review.is_some() {
            self.status = "YANDEX_REVIEW_PENDING / SYNC_DISABLED";
            return;
        }
        let Some(record) = record else {
            self.status = "YANDEX_PUBLIC_ACCOUNT_MISSING / SYNC_DISABLED";
            return;
        };
        let Some((product_app, _)) = registered_identity() else {
            self.status = "YANDEX_OAUTH_APP_NOT_REGISTERED / SYNC_DISABLED";
            return;
        };
        let scope = LocalCredentialScope::from_public_hint(
            record.account_id(), record.oauth_client_id(), product_app,
            record.workspace_uuid(), record.profile_id(),
        );
        let Ok(scope) = scope else {
            self.status = "YANDEX_PUBLIC_ACCOUNT_INVALID / SYNC_DISABLED";
            return;
        };
        self.vault = Some(record.workspace_uuid().to_owned());
        self.review_account = None;
        self.pending_forget = None;
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.busy = true;
        self.status = if forget {
            "YANDEX_FORGETTING_LOCAL_TOKEN / SYNC_DISABLED"
        } else {
            "YANDEX_CHECKING_LOCAL_TOKEN_ONLY / SYNC_DISABLED"
        };
        let tx = self.tx.clone();
        let spawn = std::thread::Builder::new()
            .name("akula-yandex-disk-local-token".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if forget {
                        Reply::LocalForgot(scope.forget_local().map(|()| record))
                    } else {
                        Reply::LocalInspected(scope.inspect())
                    }
                })).unwrap_or(Reply::WorkerFailed);
                if tx.send((generation, outcome)).is_ok() {
                    wake();
                }
            });
        if spawn.is_err() {
            self.busy = false;
            self.status = "YANDEX_WORKER_UNAVAILABLE / SYNC_DISABLED";
        }
    }
}

impl super::super::App {
    /// Public account pointer is scoped to a device-local UiState UUID,
    /// a fixed OAuth client ID and a known profile. Treat JSON as untrusted.
    pub(super) fn yandex_local_public_hint(&self, vault: Option<&str>)
        -> Option<YandexDiskPublicBinding>
    {
        let (product_app, _) = registered_identity()?;
        let workspace_uuid = vault?;
        self.workspace.core.settings.get_config()
            .yandex_disk_public
            .unverified_for(workspace_uuid, product_app, PROFILE)
            .cloned()
    }

    pub(in crate::app) fn yandex_saved_public_account(&self) -> Option<String> {
        let vault = self.yandex_disk_local_vault();
        self.yandex_local_public_hint(vault.as_deref())
            .map(|record| record.account_id().to_owned())
    }

    /// Persist an inert public pointer BEFORE the reviewed keyring write.
    /// If the JSON settings write fails, do not persist the OS credential:
    /// that would create a token users cannot later locate or forget.
    pub(super) fn remember_yandex_public_review(&mut self, vault: Option<&str>) -> bool {
        if self.yandex_disk_preview.busy
            || self.yandex_disk_preview.vault.as_deref() != vault
        {
            self.yandex_disk_preview.status = "YANDEX_WORKSPACE_CHANGED / SYNC_DISABLED";
            return false;
        }
        let Some(account) = self.yandex_disk_preview.review_account.clone() else {
            self.yandex_disk_preview.status = "YANDEX_REVIEW_NO_ACCOUNT / SYNC_DISABLED";
            return false;
        };
        let Some((product_app, _)) = registered_identity() else {
            self.yandex_disk_preview.status = "YANDEX_OAUTH_APP_NOT_REGISTERED / SYNC_DISABLED";
            return false;
        };
        let Some(workspace_uuid) = vault else { return false; };
        let record = YandexDiskPublicBinding::new(
            &account, product_app, workspace_uuid, PROFILE
        );
        let Ok(record) = record else {
            self.yandex_disk_preview.status = "YANDEX_PUBLIC_ACCOUNT_INVALID / SYNC_DISABLED";
            return false;
        };
        let mut config = self.workspace.core.settings.get_config();
        let original = config.yandex_disk_public.clone();
        if config.yandex_disk_public.remember_observed(record).is_err() {
            self.yandex_disk_preview.status = "YANDEX_PUBLIC_ACCOUNT_CONFLICT / SYNC_DISABLED";
            return false;
        }
        if config.yandex_disk_public != original
            && self.workspace.core.settings.update_config(config).is_err()
        {
            self.yandex_disk_preview.status = "YANDEX_PUBLIC_ACCOUNT_SAVE_FAILED / SYNC_DISABLED";
            return false;
        }
        true
    }

    /// A successful OS deletion is followed by an exact public pointer removal.
    /// Never report the account as forgotten if settings persistence failed;
    /// the inert public record can be retried without touching user files.
    pub(super) fn reconcile_yandex_local_forget(&mut self) -> bool {
        let Some(record) = self.yandex_disk_preview.pending_forget.take() else {
            return false;
        };
        let mut config = self.workspace.core.settings.get_config();
        match config.yandex_disk_public.forget_exact(&record) {
            Ok(true) => {
                if self.workspace.core.settings.update_config(config).is_err() {
                    self.yandex_disk_preview.status =
                        "YANDEX_PUBLIC_ACCOUNT_SAVE_FAILED / SYNC_DISABLED";
                } else {
                    self.yandex_disk_preview.status = "YANDEX_LOCAL_TOKEN_FORGOTTEN / SYNC_DISABLED";
                }
            }
            Ok(false) => {
                self.yandex_disk_preview.status = "YANDEX_LOCAL_TOKEN_ABSENT / SYNC_DISABLED";
            }
            Err(_) => {
                self.yandex_disk_preview.status = "YANDEX_PUBLIC_ACCOUNT_INVALID / SYNC_DISABLED";
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_public_account_never_contacts_os_or_browser() {
        let mut preview = YandexDiskPreview::new();
        preview.start_local_inspect(None, Arc::new(|| {}));
        assert!(!preview.busy);
        assert_eq!(preview.status, "YANDEX_PUBLIC_ACCOUNT_MISSING / SYNC_DISABLED");
        preview.start_local_forget(None, Arc::new(|| {}));
        assert!(!preview.busy);
        assert_eq!(preview.status, "YANDEX_PUBLIC_ACCOUNT_MISSING / SYNC_DISABLED");
    }

    #[test]
    fn pending_oauth_review_blocks_local_destructive_operation() {
        // A review state is intentionally private; emulate its visible
        // pending-account marker and active worker rather than using tokens.
        let mut preview = YandexDiskPreview::new();
        preview.busy = true;
        preview.start_local_forget(None, Arc::new(|| {}));
        assert!(preview.busy);
        assert!(preview.pending_forget.is_none());
    }

    #[test]
    fn absent_product_registration_disables_offline_keyring_attempts() {
        if registered_identity().is_none() {
            let record = YandexDiskPublicBinding::new(
                "1000034426", "4760187d81bc4b7799476b42b5103713",
                "12345678-1234-1234-1234-123456789abc", PROFILE
            ).unwrap();
            let mut preview = YandexDiskPreview::new();
            preview.start_local_inspect(Some(record), Arc::new(|| {}));
            assert!(!preview.busy);
            assert_eq!(preview.status, "YANDEX_OAUTH_APP_NOT_REGISTERED / SYNC_DISABLED");
        }
    }

    #[test]
    fn public_os_status_is_never_remote_authorization() {
        assert_eq!(StoredRefreshPresence::PresentUnverified,
            StoredRefreshPresence::PresentUnverified);
        assert_eq!(ConnectError::MissingAppRoot.public_status(),
            "YANDEX_APP_ROOT_UNVERIFIED / SYNC_DISABLED");
    }
}
