//! One Native background coordinator for the complete S3 pairing beta.
//! All network calls and protected key reads happen off the GUI thread.
//! No raw errors, passwords, AWS SDK objects or credentials enter UI state.

use std::{
    fs::{self, OpenOptions}, io::Write, path::{Path, PathBuf},
    sync::{mpsc::{self, Receiver, Sender}, Arc}, thread,
};
use aquilum_core::Core;
use aquilum_core::sync::s3_pair_host::{
    self, create_first_pair_profile, accept_second_pair_profile, sync_new_files_once,
};
use aquilum_core::sync::s3_pair_invite::{
    open_s3_pair_invite, seal_s3_pair_invite, MAX_PAIRING_FILE_BYTES,
};
use aquilum_core::sync::s3_pair_profile::S3PairProfile;
use crate::yandex_credentials;
use zeroize::Zeroizing;

#[cfg(target_os = "windows")]
mod custody;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PairStatus {
    #[default]
    Idle,
    Working,
    Configured,
    Exported,
    Imported,
    Transferred {
        sent: usize, received_unconfirmed: usize, needs_review: usize,
    },
    Failed,
}

/// UI-owned, non-secret observation of the one S3 pairing worker.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PairUiView {
    pub status: PairStatus,
    pub busy: bool,
    pub configured: bool,
    /// Last successful user-initiated remote verification for this workspace.
    pub verified: bool,
}

pub struct PairSession {
    rx: Receiver<PairStatus>,
    tx: Sender<PairStatus>,
    wake: Arc<dyn Fn() + Send + Sync>,
    busy: bool,
    status: PairStatus,
    active_root: Option<PathBuf>,
    verified_root: Option<PathBuf>,
}

impl PairSession {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self { rx, tx, wake, busy: false, status: PairStatus::Idle, active_root: None, verified_root: None }
    }

    pub fn status(&self) -> PairStatus { self.status }
    pub fn busy(&self) -> bool { self.busy }

    pub fn status_for(&self, root: Option<&Path>) -> PairStatus {
        if root.is_some() && self.active_root.as_deref() == root {
            self.status
        } else {
            PairStatus::Idle
        }
    }

    pub fn verified_for(&self, root: Option<&Path>) -> bool {
        root.is_some() && self.verified_root.as_deref() == root
    }

    pub fn invalidate_verification(&mut self) {
        self.verified_root = None;
    }

    fn run(&mut self, work: impl FnOnce() -> PairStatus + Send + 'static) -> bool {
        if self.busy { return false; }
        self.busy = true;
        self.status = PairStatus::Working;
        let tx = self.tx.clone();
        let wake = Arc::clone(&self.wake);
        let spawn = thread::Builder::new()
            .name("akula-s3-pair-user".into())
            .spawn(move || {
                let status = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                    .unwrap_or(PairStatus::Failed);
                if tx.send(status).is_ok() { wake(); }
            });
        if spawn.is_err() {
            self.busy = false;
            self.status = PairStatus::Failed;
            return false;
        }
        true
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(status) = self.rx.try_recv() {
            match status {
                PairStatus::Configured | PairStatus::Imported |
                    PairStatus::Transferred { received_unconfirmed: 0, needs_review: 0, .. } => {
                        self.verified_root = self.active_root.clone();
                    }
                PairStatus::Failed | PairStatus::Transferred { .. } => {
                    self.verified_root = None;
                }
                _ => {}
            }
            self.status = status;
            self.busy = false;
            changed = true;
        }
        changed
    }

    pub fn start_first_device(
        &mut self,
        core: Arc<Core>, root: PathBuf, appdata: PathBuf, bucket: String,
    ) -> bool {
        let selected = root.clone();
        let started = self.run(move || {
            let result = yandex_credentials::with_stored_keys(
                &bucket, |id, secret| {
                    let profile = with_runtime(|rt| rt.block_on(create_first_pair_profile(
                        &core, &root, &appdata, &bucket, id, secret,
                    )).map_err(|_| ()))?;
                    if !yandex_credentials::still_matches(&bucket, id, secret) {
                        return Err(());
                    }
                    // WinCred was bucket-only in the legacy setup preview.
                    // From now on, export and sync exclusively use this
                    // exact cloud identity (VaultId/prefix/genesis pin).
                    yandex_credentials::save_profile_keys(&profile, id, secret)
                        .map_err(|_| ())?;
                    let base = custody::prepare_host_base().map_err(|_| ())?;
                    s3_pair_host::provision_cooperative_s3_pair_custody(
                        &core, &root, &appdata, &base,
                    ).map_err(|_| ())?;
                    Ok(())
                },
            );
            if matches!(result, Ok(Ok(()))) {
                PairStatus::Configured
            } else {
                PairStatus::Failed
            }
        });
        if started {
            self.active_root = Some(selected);
            self.verified_root = None;
        }
        started
    }

    pub fn start_export(
        &mut self, root: PathBuf, appdata: PathBuf,
        destination: PathBuf, password: Zeroizing<String>,
    ) -> bool {
        let selected = root.clone();
        let started = self.run(move || {
            if destination.extension().and_then(|e| e.to_str()) != Some("akula-sync") {
                return PairStatus::Failed;
            }
            let result = (|| -> Result<(), ()> {
                let profile = s3_pair_host::read_pair_profile(&root, &appdata)
                    .map_err(|_| ())?.ok_or(())?;
                let encrypted = yandex_credentials::with_profile_keys(
                    &profile, |id, secret| {
                        let invite = profile.with_keys(id, secret).map_err(|_| ())?;
                        seal_s3_pair_invite(&invite, &password).map_err(|_| ())
                    },
                ).map_err(|_| ())??;
                let mut file = OpenOptions::new().write(true).create_new(true)
                    .open(&destination).map_err(|_| ())?;
                file.write_all(&encrypted).map_err(|_| ())?;
                file.sync_all().map_err(|_| ())?;
                Ok(())
            })();
            if result.is_ok() { PairStatus::Exported } else { PairStatus::Failed }
        });
        if started { self.active_root = Some(selected); }
        started
    }

    pub fn start_import(
        &mut self, core: Arc<Core>, appdata: PathBuf,
        destination_root: PathBuf, source: PathBuf,
        password: Zeroizing<String>,
        after_import: impl FnOnce(PathBuf) + Send + 'static,
    ) -> bool {
        let selected = destination_root.clone();
        let started = self.run(move || {
            let result = (|| -> Result<(), ()> {
                if !destination_root.is_dir() {
                    return Err(());
                }
                let metadata = fs::metadata(&source).map_err(|_| ())?;
                if !metadata.is_file() || metadata.len() > MAX_PAIRING_FILE_BYTES as u64 {
                    return Err(());
                }
                let sealed = fs::read(&source).map_err(|_| ())?;
                let invite = open_s3_pair_invite(&sealed, &password).map_err(|_| ())?;
                let profile = S3PairProfile::from_authenticated_invite(&invite);
                if destination_root.read_dir().map_err(|_| ())?.next().is_some()
                    && !s3_pair_host::already_imported_on_registered_root(
                        &core, &destination_root, &appdata, &profile,
                    )
                {
                    // A nonempty, unrelated workspace can never be silently
                    // combined with an imported remote root.
                    return Err(());
                }
                // A failed prior import may have registered the SAME profile.
                // Never rebind a different profile or a different remote root.
                if let Some(existing) = s3_pair_host::read_pair_profile(
                    &destination_root, &appdata,
                ).map_err(|_| ())? {
                    if existing != profile { return Err(()); }
                }
                // The entire remote trust / credential preflight runs BEFORE
                // local workspace registration, profile binding and OS locks.
                with_runtime(|rt| rt.block_on(s3_pair_host::verify_second_pair_remote(
                    &profile, &invite.access_key_id, &invite.secret_access_key,
                )).map_err(|_| ()))?;
                // A scope-specific WinCred entry cannot silently overwrite
                // a different key pair, even when the bucket name matches.
                // An interruption can leave an inert unbound key, which a
                // retry may reuse after repeating the remote verification.
                yandex_credentials::save_profile_keys(
                    &profile, &invite.access_key_id, &invite.secret_access_key,
                ).map_err(|_| ())?;
                // The selected empty directory is registered as a NEW local
                // workspace, with its own local UUID and SQLite DB.
                core.ui_state.resolve_workspace(
                    &destination_root.to_string_lossy(), crate::dates::now_ms(),
                ).map_err(|_| ())?;
                with_runtime(|rt| rt.block_on(accept_second_pair_profile(
                    &core, &destination_root, &appdata, &profile,
                    &invite.access_key_id, &invite.secret_access_key,
                )).map_err(|_| ()))?;
                let base = custody::prepare_host_base().map_err(|_| ())?;
                s3_pair_host::provision_cooperative_s3_pair_custody(
                    &core, &destination_root, &appdata, &base,
                ).map_err(|_| ())?;
                // This is only a UI preference; the authoritative selected
                // profile is in app-private storage, not in editable config.
                // Failure to save the display bucket must not report that a
                // successfully bound workspace failed to import.
                let mut settings = core.settings.get_config();
                settings.s3_onboarding.bucket = profile.bucket.clone();
                let _ = core.settings.update_config(settings);
                Ok(())
            })();
            if result.is_ok() {
                after_import(destination_root);
                PairStatus::Imported
            } else {
                PairStatus::Failed
            }
        });
        if started {
            self.active_root = Some(selected);
            self.verified_root = None;
        }
        started
    }

    pub fn start_sync_once(
        &mut self, core: Arc<Core>, root: PathBuf, appdata: PathBuf,
    ) -> bool {
        let selected = root.clone();
        let started = self.run(move || {
            let result = (|| -> Result<PairStatus, ()> {
                let profile = s3_pair_host::read_pair_profile(&root, &appdata)
                    .map_err(|_| ())?.ok_or(())?;
                // No implicit creation on SyncNow: the exact registered
                // physical root, stored cloud binding and external OS lock
                // must still match their first explicit Configure/Import.
                let host_base = custody::existing_host_base().map_err(|_| ())?;
                let scope = s3_pair_host::open_existing_cooperative_s3_pair_custody(
                    &core, &root, &appdata, &host_base,
                ).map_err(|_| ())?;
                let outcome = yandex_credentials::with_profile_keys(
                    &profile, |id, secret| {
                        with_runtime(|rt| rt.block_on(sync_new_files_once(
                            &core, &root, &appdata, Some(&scope), &profile,
                            id, secret,
                        )).map_err(|_| ()))
                    },
                ).map_err(|_| ())??;
                Ok(PairStatus::Transferred {
                    sent: outcome.uploaded_new,
                    received_unconfirmed: outcome.received_unconfirmed,
                    needs_review: outcome.needs_review,
                })
            })();
            result.unwrap_or(PairStatus::Failed)
        });
        if started { self.active_root = Some(selected); }
        started
    }
}

fn with_runtime<T>(
    action: impl FnOnce(&tokio::runtime::Runtime) -> Result<T, ()>,
) -> Result<T, ()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().map_err(|_| ())?;
    action(&rt)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_status_never_contains_aws_credentials() {
        let status = PairStatus::Transferred {
            sent: 1, received_unconfirmed: 0, needs_review: 1,
        };
        let formatted = format!("{status:?}");
        assert!(!formatted.contains("secret"));
        assert!(formatted.contains("needs_review"));
    }
}
