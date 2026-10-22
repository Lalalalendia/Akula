//! One Native background coordinator for the complete S3 pairing beta.
//! All network calls and protected key reads happen off the GUI thread.
//! No raw errors, passwords, AWS SDK objects or credentials enter UI state.

use std::{
    fs::{self, OpenOptions}, io::Write, path::PathBuf,
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

pub struct PairSession {
    rx: Receiver<PairStatus>,
    tx: Sender<PairStatus>,
    wake: Arc<dyn Fn() + Send + Sync>,
    busy: bool,
    status: PairStatus,
}

impl PairSession {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self { rx, tx, wake, busy: false, status: PairStatus::Idle }
    }

    pub fn status(&self) -> PairStatus { self.status }
    pub fn busy(&self) -> bool { self.busy }

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
        self.run(move || {
            let result = yandex_credentials::with_stored_keys(
                &bucket, |id, secret| {
                    let outcome = with_runtime(|rt| rt.block_on(create_first_pair_profile(
                        &core, &root, &appdata, &bucket, id, secret,
                    )).map(|_| ()));
                    if outcome.is_ok()
                        && !yandex_credentials::still_matches(&bucket, id, secret)
                    {
                        Err(())
                    } else { outcome }
                },
            );
            if matches!(result, Ok(Ok(()))) {
                PairStatus::Configured
            } else {
                PairStatus::Failed
            }
        })
    }

    pub fn start_export(
        &mut self, root: PathBuf, appdata: PathBuf,
        destination: PathBuf, password: Zeroizing<String>,
    ) -> bool {
        self.run(move || {
            if destination.extension().and_then(|e| e.to_str()) != Some("akula-sync") {
                return PairStatus::Failed;
            }
            let result = (|| -> Result<(), ()> {
                let profile = s3_pair_host::read_pair_profile(&root, &appdata)
                    .map_err(|_| ())?.ok_or(())?;
                let encrypted = yandex_credentials::with_stored_keys(
                    &profile.bucket, |id, secret| {
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
        })
    }

    pub fn start_import(
        &mut self, core: Arc<Core>, appdata: PathBuf,
        destination_root: PathBuf, source: PathBuf,
        password: Zeroizing<String>,
        after_import: impl FnOnce(PathBuf) + Send + 'static,
    ) -> bool {
        self.run(move || {
            let result = (|| -> Result<(), ()> {
                if !destination_root.is_dir()
                    || destination_root.read_dir().map_err(|_| ())?.next().is_some()
                {
                    return Err(());
                }
                let metadata = fs::metadata(&source).map_err(|_| ())?;
                if !metadata.is_file() || metadata.len() > MAX_PAIRING_FILE_BYTES as u64 {
                    return Err(());
                }
                let sealed = fs::read(&source).map_err(|_| ())?;
                let invite = open_s3_pair_invite(&sealed, &password).map_err(|_| ())?;
                let profile = S3PairProfile::from_authenticated_invite(&invite);
                // The selected empty directory is registered as a NEW local
                // workspace. The cloud UUID is never a local workspace UUID.
                core.ui_state.resolve_workspace(
                    &destination_root.to_string_lossy(), crate::dates::now_ms(),
                ).map_err(|_| ())?;
                with_runtime(|rt| rt.block_on(accept_second_pair_profile(
                    &core, &destination_root, &appdata, &profile,
                    &invite.access_key_id, &invite.secret_access_key,
                )).map_err(|_| ()))?;
                yandex_credentials::save_imported_keys(
                    &profile.bucket, &invite.access_key_id, &invite.secret_access_key,
                ).map_err(|_| ())?;
                // Only the non-secret selected bucket is reflected in Native
                // settings. Password, profile keys and SQLite state are never
                // serialized into AppConfig or copied to the UI.
                let mut settings = core.settings.get_config();
                settings.s3_onboarding.bucket = profile.bucket.clone();
                core.settings.update_config(settings).map_err(|_| ())?;
                Ok(())
            })();
            if result.is_ok() {
                after_import(destination_root);
                PairStatus::Imported
            } else {
                PairStatus::Failed
            }
        })
    }

    pub fn start_sync_once(
        &mut self, core: Arc<Core>, root: PathBuf, appdata: PathBuf,
    ) -> bool {
        self.run(move || {
            let result = (|| -> Result<PairStatus, ()> {
                let profile = s3_pair_host::read_pair_profile(&root, &appdata)
                    .map_err(|_| ())?.ok_or(())?;
                let outcome = yandex_credentials::with_stored_keys(
                    &profile.bucket, |id, secret| {
                        with_runtime(|rt| rt.block_on(sync_new_files_once(
                            &core, &root, &appdata, None, &profile, id, secret,
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
        })
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
