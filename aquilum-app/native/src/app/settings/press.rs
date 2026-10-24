use super::*;
use std::path::Path;
use crate::i18n::t;
impl App {
    pub(super) fn press(&mut self, dialog: &mut SettingsDialog, action: Action) -> Effect {
        let core = &self.workspace.core;
        let workspace = self.tree.root().map(Path::to_path_buf);
        match action {
            // This handler is reached only for ordinary settings buttons.
            // Review actions require the separately confirmed modal flow.
            Action::OpenCloudProvider(_) => Effect::None,
            Action::InspectRecovery(_) | Action::RequestRecoveryReview(_) => Effect::None,
            #[cfg(feature = "dropbox-readonly-preview")]
            a @ (Action::DropboxLoginAccount | Action::DropboxResumeAccount | Action::DropboxForgetAccount | Action::DropboxConnectReadOnly | Action::DropboxResumeReadOnly) => self.press_dropbox(a, dialog.section),
            Action::OpenTrash => {
                if let Some(workspace) = workspace {
                    match gate::ensure_trash(core, &workspace) {
                        Ok(path) => system::open(&path),
                        Err(error) => report!("корзина не открыта: {error:?}"),
                    }
                }
                Effect::None
            }
            Action::Restore(id) => {
                let Some(workspace) = workspace else { return Effect::None };
                if let Err(error) = gate::restore_deletion(core, &workspace, &id) {
                    report!("заметка не восстановлена: {error:?}");
                }
                Effect::Content
            }
            Action::TrashPage(page) => {
                dialog.trash_page = page;
                Effect::Content
            }
            Action::CreateTemplates => {
                let Some(workspace) = workspace else { return Effect::None };
                crate::app::templates::write_starters(core, &schema::templates_path(&workspace, &core.settings.get_config().templates.folder));
                Effect::Content
            }
            Action::CheckUpdates => {
                self.updater.check();
                Effect::Content
            }
            Action::InstallUpdate => {
                self.updater.install();
                Effect::Content
            }
            Action::NewToken => self.apply(Setter::Config(|c, _| c.mcp.token = schema::new_token()), Value::Bool(true)),
            Action::Copy(index, code) => {
                match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(code)) {
                    Ok(()) => dialog.copied = Some(index),
                    Err(error) => report!("не скопировано: {error}"),
                }
                Effect::Content
            }
            Action::OpenUrl(url) => {
                system::open_url(url);
                Effect::None
            }
            Action::StoreYandexKeys if dialog.section == Section::Sync => {
                let bucket = core.settings.get_config().s3_onboarding.bucket;
                let result = crate::yandex_credentials::prompt_and_save(
                    &bucket,
                    &t("settings.yandex.promptTitle"),
                    &t("settings.yandex.promptText"),
                );
                if matches!(result, Ok(crate::yandex_credentials::SaveOutcome::Saved)) {
                    self.yandex_probe.credentials_changed();
                    #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
                    self.s3_pair.invalidate_verification();
                }
                let key = match result {
                    Ok(crate::yandex_credentials::SaveOutcome::Saved) =>
                        "settings.yandex.keysSaved",
                    Ok(crate::yandex_credentials::SaveOutcome::Cancelled) =>
                        "settings.yandex.keysCancelled",
                    Err(crate::yandex_credentials::VaultError::InvalidKeys) =>
                        "settings.yandex.keysInvalid",
                    Err(_) => "settings.yandex.keysStoreFailed",
                };
                // Only a fixed localized outcome is reported to the UI.
                // Never print a key, username or raw Windows error.
                dialog.yandex_notice = Some(t(key));
                Effect::Content
            }
            Action::RemoveYandexKeys if dialog.section == Section::Sync => {
                let bucket = core.settings.get_config().s3_onboarding.bucket;
                let confirmed = rfd::MessageDialog::new()
                    .set_title(t("settings.yandex.keysConfirmTitle"))
                    .set_description(t("settings.yandex.keysConfirmText"))
                    .set_level(rfd::MessageLevel::Warning)
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show() == rfd::MessageDialogResult::Yes;
                let key = if !confirmed {
                    "settings.yandex.keysRemoveCancelled"
                } else {
                    match crate::yandex_credentials::remove(&bucket) {
                        Ok(()) => {
                            self.yandex_probe.credentials_changed();
                            #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
                            self.s3_pair.invalidate_verification();
                            "settings.yandex.keysRemoved"
                        },
                        Err(_) => "settings.yandex.keysRemoveFailed",
                    }
                };
                dialog.yandex_notice = Some(t(key));
                Effect::Content
            }
            // The preview is never compiled into ordinary installed builds.
            // Worker reads keys itself; the GUI captures ONLY the bucket.
            #[cfg(all(target_os = "windows", feature = "yandex-s3-connection-preview"))]
            Action::CheckYandexConnection if dialog.section == Section::Sync => {
                let bucket = core.settings.get_config().s3_onboarding.bucket;
                self.yandex_probe.set_bucket(&bucket);
                let present = crate::yandex_credentials::present(&bucket).unwrap_or(false);
                let selected = bucket.clone();
                let result = self.yandex_probe.start(present, move || {
                    crate::yandex_probe_worker::check(&selected)
                });
                dialog.yandex_notice = result.err().map(|reason| t(match reason {
                    crate::yandex_probe_session::StartError::InvalidBucket =>
                        "settings.yandex.keysNeedBucket",
                    crate::yandex_probe_session::StartError::MissingKeys =>
                        "settings.yandex.probeMissingKeys",
                    crate::yandex_probe_session::StartError::Busy =>
                        "settings.yandex.probeChecking",
                    crate::yandex_probe_session::StartError::SequenceExhausted |
                    crate::yandex_probe_session::StartError::WorkerUnavailable =>
                        "settings.yandex.probeServer",
                }));
                Effect::Content
            }
            #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
            Action::ConfigureS3Pair if dialog.section == Section::Sync => {
                if let Some(root) = workspace {
                    let bucket = core.settings.get_config().s3_onboarding.bucket;
                    let started = self.s3_pair.start_first_device(
                        std::sync::Arc::clone(core), root,
                        self.workspace.data_dir.clone(), bucket,
                    );
                    dialog.yandex_notice = Some(t(if started {
                        "settings.yandex.pairQueued"
                    } else { "settings.yandex.pairBusy" }));
                }
                Effect::Content
            }
            #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
            Action::ExportS3Pair if dialog.section == Section::Sync => {
                let Some(root) = workspace else { return Effect::None };
                let pass = match crate::s3_pair_password::ask_export_password() {
                    Ok(Some(pass)) => pass,
                    _ => return Effect::None,
                };
                let destination = rfd::FileDialog::new()
                    .add_filter("Akula S3", &["akula-sync"])
                    .set_file_name("Akula.akula-sync")
                    .save_file();
                if let Some(file) = destination {
                    let started = self.s3_pair.start_export(
                        root, self.workspace.data_dir.clone(), file, pass,
                    );
                    dialog.yandex_notice = Some(t(if started {
                        "settings.yandex.pairQueued"
                    } else { "settings.yandex.pairBusy" }));
                }
                Effect::Content
            }
            #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
            Action::ImportS3Pair if dialog.section == Section::Sync => {
                let source = rfd::FileDialog::new()
                    .add_filter("Akula S3", &["akula-sync"])
                    .pick_file();
                let Some(source) = source else { return Effect::None };
                let password = match crate::s3_pair_password::ask_import_password() {
                    Ok(Some(pass)) => pass,
                    _ => return Effect::None,
                };
                // Require an empty, explicitly selected destination. Never
                // copy a cloud profile over existing local notes.
                let destination = rfd::FileDialog::new().pick_folder();
                if let Some(root) = destination {
                    let channel = self.picked_tx.clone();
                    let wake = std::sync::Arc::clone(&self.workspace.wake);
                    let started = self.s3_pair.start_import(
                        std::sync::Arc::clone(core),
                        self.workspace.data_dir.clone(), root, source, password,
                        move |new_root| {
                            if channel.send(crate::app::book::Picked::Workspace(new_root)).is_ok() {
                                wake();
                            }
                        },
                    );
                    dialog.yandex_notice = Some(t(if started {
                        "settings.yandex.pairQueued"
                    } else { "settings.yandex.pairBusy" }));
                }
                Effect::Content
            }
            #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
            Action::SyncS3PairOnce if dialog.section == Section::Sync => {
                if let Some(root) = workspace {
                    let started = self.s3_pair.start_sync_once(
                        std::sync::Arc::clone(core), root,
                        self.workspace.data_dir.clone(),
                    );
                    dialog.yandex_notice = Some(t(if started {
                        "settings.yandex.pairQueued"
                    } else { "settings.yandex.pairBusy" }));
                }
                Effect::Content
            }
            #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
            Action::ConfigureS3Pair | Action::ExportS3Pair |
                Action::ImportS3Pair | Action::SyncS3PairOnce => Effect::None,
            Action::StoreYandexKeys | Action::RemoveYandexKeys | Action::CheckYandexConnection => Effect::None,
            other => self.handle_server_button(dialog, other),
        }
    }
}
