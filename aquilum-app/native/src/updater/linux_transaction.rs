//! RELEASE-LINUX-RUNTIME-GATE-01: Linux AppImage transaction engine, compiled
//! with the native updater but intentionally denied release admission.
//! Tested with real cross-process SIGKILL rename/fsync recovery. It does not
//! establish publisher identity, power-loss durability or a real GUI startup ACK.
#![cfg(target_os = "linux")]

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

const LOCK_NAME: &str = ".akula-appimage-update.lock";
const PREFIX: &str = ".akula-appimage-update.";
const RECEIPT: &str = "prepared.v1";
pub(crate) const CANDIDATE: &str = "candidate.AppImage";
const PREVIOUS: &str = "previous.AppImage";
const FAILED: &str = "failed.AppImage";
const MAX_PAYLOAD: usize = 512 * 1024 * 1024;

const LOCK_EX: i32 = 2;
const LOCK_NB: i32 = 4;
const LOCK_UN: i32 = 8;
unsafe extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}

fn bad(reason: &str) -> String {
    format!("AKULA_LINUX_ROLLBACK_HOLD: {reason}")
}

fn sync_dir(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|e| format!("directory sync failed at {}: {e}", path.display()))
}

fn is_real_dir(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_dir() && !meta.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(bad("directory is not a real directory")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

fn is_real_file(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(meta)
            if meta.file_type().is_file()
                && !meta.file_type().is_symlink()
                && meta.nlink() == 1 =>
        {
            Ok(true)
        }
        Ok(_) => Err(bad("file is a symlink, a hardlink, or a non-file")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

fn valid_nonce(nonce: &str) -> bool {
    nonce.len() == 32 && nonce.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Debug, PartialEq, Eq)]
enum Recovery {
    Clean,
    PreviousRestored,
    PendingUnacknowledged,
}

pub(crate) struct Gate {
    _lock: File,
    parent: PathBuf,
    active: PathBuf,
}

// Release the same advisory OS lock explicitly before closing the descriptor.
// SIGKILL remains covered by kernel fd teardown; no lockfile is unlinked.
impl Drop for Gate {
    fn drop(&mut self) {
        let _ = unsafe { flock(self._lock.as_raw_fd(), LOCK_UN) };
    }
}

impl Gate {
    pub(crate) fn acquire(active: &Path) -> Result<Self, String> {
        let name = active.file_name().and_then(OsStr::to_str)
            .ok_or_else(|| bad("invalid installed AppImage filename"))?;
        if !name.starts_with("akula-app_") || !name.ends_with("_x86_64.AppImage")
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(bad("refusing non-Akula AppImage destination"));
        }
        let parent = active.parent().ok_or_else(|| bad("missing installation directory"))?;
        if !is_real_dir(parent)? {
            return Err(bad("installation directory missing"));
        }
        let _ = is_real_file(active)?; // Missing only permitted for cold recovery.
        let lock_path = parent.join(LOCK_NAME);
        let _ = is_real_file(&lock_path)?;
        let lock = OpenOptions::new().append(true).create(true).open(&lock_path)
            .map_err(|e| e.to_string())?;
        if unsafe { flock(lock.as_raw_fd(), LOCK_EX | LOCK_NB) } != 0 {
            return Err(format!("{}: {}", bad("another updater holds OS lock"), io::Error::last_os_error()));
        }
        // The lock path must still identify the held inode after acquiring it.
        let current = fs::symlink_metadata(&lock_path).map_err(|e| e.to_string())?;
        let held = lock.metadata().map_err(|e| e.to_string())?;
        if !current.file_type().is_file() || current.file_type().is_symlink()
            || current.ino() != held.ino() || current.dev() != held.dev()
        {
            return Err(bad("shared installer lock path changed"));
        }
        Ok(Self { _lock: lock, parent: parent.to_path_buf(), active: active.to_path_buf() })
    }

    fn stages(&self) -> Result<Vec<PathBuf>, String> {
        let mut found = Vec::new();
        for entry in fs::read_dir(&self.parent).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                return Err(bad("unreadable name under installation directory"));
            };
            if name == LOCK_NAME || !name.starts_with(PREFIX) {
                continue;
            }
            let nonce = &name[PREFIX.len()..];
            if !valid_nonce(nonce) || !is_real_dir(&entry.path())? {
                return Err(bad("unknown or aliased update generation"));
            }
            found.push(entry.path());
            if found.len() > 1 {
                return Err(bad("multiple update generations require manual review"));
            }
        }
        Ok(found)
    }

    fn sole_stage(&self, stage: &Path) -> Result<(), String> {
        let items = self.stages()?;
        if items.len() != 1 || items[0] != stage {
            return Err(bad("stage does not match the one permitted generation"));
        }
        let nonce = stage.file_name().and_then(OsStr::to_str)
            .and_then(|name| name.strip_prefix(PREFIX))
            .ok_or_else(|| bad("invalid stage"))?;
        let receipt = stage.join(RECEIPT);
        if !is_real_file(&receipt)? {
            return Err(bad("missing prepared receipt"));
        }
        let bytes = fs::read(&receipt).map_err(|e| e.to_string())?;
        if bytes.len() > 512 {
            return Err(bad("oversized stage receipt"));
        }
        let name = self.active.file_name().and_then(OsStr::to_str)
            .ok_or_else(|| bad("invalid active name"))?;
        let expected = format!("AKULA-APPIMAGE-STAGED-V1\n{nonce}\n{name}\n");
        if bytes != expected.as_bytes() {
            return Err(bad("stage generation receipt differs"));
        }
        for item in fs::read_dir(stage).map_err(|e| e.to_string())? {
            let item = item.map_err(|e| e.to_string())?;
            let name = item.file_name();
            if ![RECEIPT, CANDIDATE, PREVIOUS, FAILED]
                .iter().any(|known| name == OsStr::new(known))
                || !is_real_file(&item.path())?
            {
                return Err(bad("unexpected or aliased stage entry"));
            }
        }
        Ok(())
    }

    pub(crate) fn prepare(&self, nonce: &str, payload: &[u8]) -> Result<PathBuf, String> {
        if !valid_nonce(nonce) || payload.is_empty() || payload.len() > MAX_PAYLOAD {
            return Err(bad("invalid generation or bounded payload"));
        }
        if !is_real_file(&self.active)? || !self.stages()?.is_empty() {
            return Err(bad("must have a single clean installed generation"));
        }
        let stage = self.parent.join(format!("{PREFIX}{nonce}"));
        fs::create_dir(&stage).map_err(|e| e.to_string())?;
        let candidate = stage.join(CANDIDATE);
        let mut file = OpenOptions::new().write(true).create_new(true).open(&candidate)
            .map_err(|e| e.to_string())?;
        file.write_all(payload).map_err(|e| e.to_string())?;
        file.set_permissions(fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        let mut receipt = OpenOptions::new().write(true).create_new(true)
            .open(stage.join(RECEIPT)).map_err(|e| e.to_string())?;
        writeln!(receipt, "AKULA-APPIMAGE-STAGED-V1\n{nonce}\n{}",
            self.active.file_name().and_then(OsStr::to_str).unwrap_or(""))
            .map_err(|e| e.to_string())?;
        receipt.sync_all().map_err(|e| e.to_string())?;
        sync_dir(&stage)?;
        sync_dir(&self.parent)?;
        Ok(stage)
    }

    pub(crate) fn backup_old(&self, stage: &Path) -> Result<(), String> {
        self.sole_stage(stage)?;
        if !is_real_file(&self.active)? || !is_real_file(&stage.join(CANDIDATE))?
            || is_real_file(&stage.join(PREVIOUS))?
        {
            return Err(bad("cannot back up old generation"));
        }
        fs::rename(&self.active, stage.join(PREVIOUS)).map_err(|e| e.to_string())?;
        sync_dir(&self.parent)?;
        sync_dir(stage)?;
        Ok(())
    }

    pub(crate) fn publish_new(&self, stage: &Path) -> Result<(), String> {
        self.sole_stage(stage)?;
        if !is_real_file(&stage.join(PREVIOUS))?
            || !is_real_file(&stage.join(CANDIDATE))?
            || is_real_file(&self.active)?
        {
            return Err(bad("publication needs previous backup and no current destination"));
        }
        fs::rename(stage.join(CANDIDATE), &self.active).map_err(|e| e.to_string())?;
        sync_dir(stage)?;
        sync_dir(&self.parent)?;
        Ok(())
    }

    fn recover(&self) -> Result<Recovery, String> {
        let stages = self.stages()?;
        if stages.is_empty() {
            return if is_real_file(&self.active)? {
                Ok(Recovery::Clean)
            } else {
                Err(bad("no installed image and no previous backup"))
            };
        }
        let stage = &stages[0];
        self.sole_stage(stage)?;
        let previous = stage.join(PREVIOUS);
        let backed = is_real_file(&previous)?;
        let installed = is_real_file(&self.active)?;
        if backed && !installed {
            fs::rename(&previous, &self.active).map_err(|e| e.to_string())?;
            sync_dir(stage)?;
            sync_dir(&self.parent)?;
            return Ok(Recovery::PreviousRestored);
        }
        if backed && installed {
            return Ok(Recovery::PendingUnacknowledged);
        }
        Err(bad("ambiguous staged update preserved for manual review"))
    }

    /// Explicit cold rollback. Caller must independently establish that the
    /// new application has stopped and no generation-bound GUI ACK was issued.
    /// It never deletes the failed candidate; backup survives until final rename.
    pub(crate) fn rollback_unacknowledged(&self, stage: &Path) -> Result<(), String> {
        self.sole_stage(stage)?;
        if !is_real_file(&stage.join(PREVIOUS))?
            || !is_real_file(&self.active)?
            || is_real_file(&stage.join(CANDIDATE))?
            || is_real_file(&stage.join(FAILED))?
        {
            return Err(bad("rollback preconditions or generation changed"));
        }
        fs::rename(&self.active, stage.join(FAILED)).map_err(|e| e.to_string())?;
        sync_dir(stage)?;
        sync_dir(&self.parent)?;
        fs::rename(stage.join(PREVIOUS), &self.active).map_err(|e| e.to_string())?;
        sync_dir(stage)?;
        sync_dir(&self.parent)?;
        Ok(())
    }
}


/** The shipped Linux updater must not mutate user files until it has both a
 * production publisher-bound signed admission and a generation-specific GUI
 * startup acknowledgement protocol. Neither is provided yet.
 *
 * This deliberately callable runtime entrypoint is what apply.rs invokes.
 * Its refusal occurs before shutdown, lock acquisition, staging or renames.
 * The transaction engine below is compiled but has no live admission path.
 */
pub(crate) fn guarded_runtime_apply(payload: &[u8], before_exit: impl FnOnce()) -> Result<(), String> {
    let _ = (payload, before_exit);
    Err(bad("signed Akula candidate admission and first-render ACK are not approved"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::Duration;

    const NONCE: &str = "0123456789abcdef0123456789abcdef";
    static NEXT: AtomicU64 = AtomicU64::new(0);
    const OLD: &[u8] = b"old verified Akula AppImage bytes";
    const NEW: &[u8] = b"new synthetic AppImage bytes";
    const AQUILUM: &[u8] = b"existing unrelated Aquilum installation witness";

    struct Fixture {
        root: PathBuf,
        app: PathBuf,
        aquilum: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let number = NEXT.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!("akula-linux-rollback-{}-{number}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            let app = root.join("akula-app_0.3.9_x86_64.AppImage");
            let aquilum = root.join("Aquilum.AppImage");
            fs::write(&app, OLD).unwrap();
            fs::write(&aquilum, AQUILUM).unwrap();
            Self { root, app, aquilum }
        }
        fn gate(&self) -> Gate {
            Gate::acquire(&self.app).unwrap()
        }
        fn stage(&self) -> PathBuf {
            self.root.join(format!("{PREFIX}{NONCE}"))
        }
        fn assert_untouched(&self) {
            assert_eq!(fs::read(&self.aquilum).unwrap(), AQUILUM);
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }


    #[test]
    fn guarded_runtime_apply_refuses_payload_before_shutdown() {
        let mut shutdown_called = false;
        let failure = guarded_runtime_apply(b"forged unsigned update", || {
            shutdown_called = true;
        }).unwrap_err();
        assert!(failure.contains("HOLD"));
        assert!(!shutdown_called, "refusal must happen before shutdown or writes");
    }

    #[test]
    fn clean_installation_and_exclusive_os_lock() {
        let f = Fixture::new();
        let gate = f.gate();
        assert_eq!(gate.recover().unwrap(), Recovery::Clean);
        assert!(Gate::acquire(&f.app).is_err(), "second independently opened fd must not bypass flock");
        drop(gate);
        let reacquired = Gate::acquire(&f.app);
        assert!(
            reacquired.is_ok(),
            "independent Linux fd must reacquire after explicit flock unlock: {:?}",
            reacquired.as_ref().err()
        );
        drop(reacquired);
        f.assert_untouched();
    }

    #[test]
    fn staged_files_cannot_be_published_without_renaming_previous_generation() {
        let f = Fixture::new();
        let gate = f.gate();
        let stage = gate.prepare(NONCE, NEW).unwrap();
        assert!(gate.publish_new(&stage).is_err());
        assert_eq!(fs::read(&f.app).unwrap(), OLD);
        assert!(gate.recover().is_err(), "abandoned stage remains HOLD");
        assert_eq!(fs::read(stage.join(CANDIDATE)).unwrap(), NEW);
        f.assert_untouched();
    }

    #[test]
    fn interrupted_between_old_backup_and_new_publish_restores_previous() {
        let f = Fixture::new();
        let gate = f.gate();
        let stage = gate.prepare(NONCE, NEW).unwrap();
        gate.backup_old(&stage).unwrap();
        assert!(!f.app.exists());
        drop(gate);
        let cold = f.gate();
        assert_eq!(cold.recover().unwrap(), Recovery::PreviousRestored);
        assert_eq!(fs::read(&f.app).unwrap(), OLD);
        assert_eq!(fs::read(stage.join(CANDIDATE)).unwrap(), NEW);
        f.assert_untouched();
    }

    #[test]
    fn published_generation_requires_explicit_cold_rollback_not_implicit_success() {
        let f = Fixture::new();
        let gate = f.gate();
        let stage = gate.prepare(NONCE, NEW).unwrap();
        gate.backup_old(&stage).unwrap();
        gate.publish_new(&stage).unwrap();
        assert_eq!(gate.recover().unwrap(), Recovery::PendingUnacknowledged);
        assert_eq!(fs::read(&f.app).unwrap(), NEW);
        assert_eq!(fs::read(stage.join(PREVIOUS)).unwrap(), OLD);
        gate.rollback_unacknowledged(&stage).unwrap();
        assert_eq!(fs::read(&f.app).unwrap(), OLD);
        assert_eq!(fs::read(stage.join(FAILED)).unwrap(), NEW);
        assert!(gate.recover().is_err(), "restored old must not make unresolved stage disappear");
        f.assert_untouched();
    }

    #[test]
    fn multiple_generations_and_symlinked_payloads_refuse_recovery() {
        let f = Fixture::new();
        let gate = f.gate();
        let stage = gate.prepare(NONCE, NEW).unwrap();
        fs::create_dir(f.root.join(format!("{PREFIX}aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"))).unwrap();
        assert!(gate.recover().is_err());
        fs::remove_dir(f.root.join(format!("{PREFIX}aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"))).unwrap();
        fs::remove_file(stage.join(CANDIDATE)).unwrap();
        std::os::unix::fs::symlink(&f.aquilum, stage.join(CANDIDATE)).unwrap();
        assert!(gate.backup_old(&stage).is_err());
        assert_eq!(fs::read(&f.app).unwrap(), OLD);
        f.assert_untouched();
    }

    #[test]
    fn foreign_installation_and_symlinked_lock_are_rejected() {
        let f = Fixture::new();
        assert!(Gate::acquire(&f.aquilum).is_err());
        let lock = f.root.join(LOCK_NAME);
        std::os::unix::fs::symlink(&f.aquilum, &lock).unwrap();
        assert!(Gate::acquire(&f.app).is_err());
        f.assert_untouched();
    }

    #[test]
    fn child_crash_at_transaction_boundary() {
        let Ok(phase) = std::env::var("AKULA_LINUX_CRASH_PHASE") else { return };
        let root = PathBuf::from(std::env::var("AKULA_LINUX_CRASH_ROOT").unwrap());
        let active = root.join("akula-app_0.3.9_x86_64.AppImage");
        let gate = Gate::acquire(&active).unwrap();
        let stage = gate.prepare(NONCE, NEW).unwrap();
        if phase == "after-backup" || phase == "after-publish" || phase == "during-rollback" {
            gate.backup_old(&stage).unwrap();
        }
        if phase == "after-publish" || phase == "during-rollback" {
            gate.publish_new(&stage).unwrap();
        }
        if phase == "during-rollback" {
            fs::rename(&active, stage.join(FAILED)).unwrap();
            sync_dir(&root).unwrap();
            sync_dir(&stage).unwrap();
        }
        let marker = root.join("child-ready");
        let mut file = OpenOptions::new().create_new(true).write(true).open(marker).unwrap();
        file.write_all(phase.as_bytes()).unwrap();
        file.sync_all().unwrap();
        sync_dir(&root).unwrap();
        // The parent sends a real SIGKILL; no Rust unwinding or Drop runs here.
        loop { thread::sleep(Duration::from_secs(3)); }
    }

    fn kill_child_at(phase: &str) -> Fixture {
        let f = Fixture::new();
        let exe = std::env::current_exe().unwrap();
        let list = Command::new(&exe).arg("--list").output().unwrap();
        assert!(list.status.success());
        let listing = String::from_utf8_lossy(&list.stdout);
        // Select only the helper from THIS module. The macOS transaction
        // harness registers another helper with the same short name.
        let suite = module_path!()
            .split_once("::")
            .map_or(module_path!(), |(_, rest)| rest);
        let exact = format!("{suite}::child_crash_at_transaction_boundary");
        let helper = listing.lines()
            .filter_map(|line| line.strip_suffix(": test"))
            .find(|name| *name == exact.as_str())
            .expect("this Linux crash subprocess helper was not registered");
        let mut child = Command::new(exe)
            .arg("--exact").arg(helper).arg("--nocapture")
            .env("AKULA_LINUX_CRASH_PHASE", phase)
            .env("AKULA_LINUX_CRASH_ROOT", &f.root)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn().unwrap();
        let mut reached = false;
        for _ in 0..240 {
            if f.root.join("child-ready").exists() {
                reached = true;
                break;
            }
            if child.try_wait().unwrap().is_some() { break; }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = child.kill(); // Unix Child::kill sends SIGKILL.
        let _ = child.wait();
        assert!(reached, "child did not reach and persist crash marker for phase {phase}");
        f
    }

    #[test]
    fn actual_sigkill_after_old_backup_then_independent_cold_recovery() {
        let f = kill_child_at("after-backup");
        assert!(!f.app.exists());
        let cold = f.gate(); // Must reacquire after killed OS process.
        assert_eq!(cold.recover().unwrap(), Recovery::PreviousRestored);
        assert_eq!(fs::read(&f.app).unwrap(), OLD);
        f.assert_untouched();
    }

    #[test]
    fn actual_sigkill_after_new_publish_preserves_both_generations() {
        let f = kill_child_at("after-publish");
        let cold = f.gate();
        assert_eq!(cold.recover().unwrap(), Recovery::PendingUnacknowledged);
        assert_eq!(fs::read(&f.app).unwrap(), NEW);
        assert_eq!(fs::read(f.stage().join(PREVIOUS)).unwrap(), OLD);
        cold.rollback_unacknowledged(&f.stage()).unwrap();
        assert_eq!(fs::read(&f.app).unwrap(), OLD);
        assert_eq!(fs::read(f.stage().join(FAILED)).unwrap(), NEW);
        f.assert_untouched();
    }

    #[test]
    fn actual_sigkill_halfway_through_rollback_restores_previous() {
        let f = kill_child_at("during-rollback");
        assert!(!f.app.exists());
        let cold = f.gate();
        assert_eq!(cold.recover().unwrap(), Recovery::PreviousRestored);
        assert_eq!(fs::read(&f.app).unwrap(), OLD);
        assert_eq!(fs::read(f.stage().join(FAILED)).unwrap(), NEW);
        f.assert_untouched();
    }
}
