//! Cooperative user-local macOS updater transaction.
//! Holds the identical persistent BSD advisory lock as shell install/uninstall.
//! NOT a hostile same-UID isolation boundary or a signed release admission.
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

const LOCK_EX: i32 = 2;
const LOCK_NB: i32 = 4;
const LOCK_UN: i32 = 8;
const LOCK_NAME: &str = ".akula-install.lock";
const STAGE_PREFIX: &str = ".akula-install.";
const APP_NAME: &str = "Aquilum.app";
const OLD_NAME: &str = "Aquilum.previous.app";

unsafe extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}

fn valid_directory(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_dir() && !meta.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(format!("directory is aliased or replaced: {}", path.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

fn missing(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(e.to_string()),
    }
}

#[derive(Debug)]
pub(super) struct AppGate {
    _lock: File,
    parent: PathBuf,
    bundle: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Recovery {
    Clean,
    PreviousRestored,
}

impl Drop for AppGate {
    fn drop(&mut self) {
        // Release the lock even if a forked helper retained a duplicated fd.
        let _ = unsafe { flock(self._lock.as_raw_fd(), LOCK_UN) };
    }
}

impl AppGate {
    /// Refuse /Applications or translocated apps. Only user-local
    /// ~/Applications shares a lock with the Akula shell installer.
    pub(super) fn acquire(bundle: &Path, home: &Path) -> Result<Self, String> {
        if bundle.file_name() != Some(OsStr::new(APP_NAME)) {
            return Err("updater destination must be Aquilum.app".into());
        }
        let parent = bundle.parent().ok_or("updater has no app parent")?;
        let expected = home.join("Applications");
        if !valid_directory(parent)? || !valid_directory(&expected)? {
            return Err("user-local Applications directory unavailable".into());
        }
        if fs::canonicalize(parent).map_err(|e| e.to_string())?
            != fs::canonicalize(expected).map_err(|e| e.to_string())?
        {
            return Err("app is outside the shared user-local installer authority".into());
        }
        // A missing bundle may mean an interrupted old->backup rename, so
        // do NOT deny entry before the recovery check under the held lock.
        let _ = valid_directory(bundle)?;
        let path = parent.join(LOCK_NAME);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_file() && !meta.file_type().is_symlink() => {}
            Ok(_) => return Err("shared installer lock is aliased or non-file".into()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        // BSD flock(2) matches macOS /usr/bin/lockf's inherited-fd mode.
        if unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } != 0 {
            return Err(format!(
                "another Akula installer/updater holds the shared lock: {}",
                io::Error::last_os_error()
            ));
        }
        let _ = valid_directory(bundle)?;
        Ok(Self {
            _lock: file,
            parent: parent.to_path_buf(),
            bundle: bundle.to_path_buf(),
        })
    }

    fn stages(&self) -> Result<Vec<PathBuf>, String> {
        let mut stages = Vec::new();
        for item in fs::read_dir(&self.parent).map_err(|e| e.to_string())? {
            let item = item.map_err(|e| e.to_string())?;
            let name = item.file_name();
            let Some(name) = name.to_str() else {
                return Err("unreadable Applications entry name; refusing recovery".into());
            };
            if name == LOCK_NAME || !name.starts_with(STAGE_PREFIX) {
                continue;
            }
            if !valid_directory(&item.path())? {
                return Err("interrupted updater directory disappeared".into());
            }
            stages.push(item.path());
            if stages.len() > 1 {
                return Err("multiple interrupted installer generations: HOLD".into());
            }
        }
        Ok(stages)
    }

    /// Read-only generation query under the same held BSD app lock.
    /// Never auto-clear a staged backup from the new process.
    // Used by the real startup_ack sibling module, not by this module's
    // standalone rustc --test harness that compiles with -D warnings.
    #[allow(dead_code)]
    pub(super) fn startup_stage(&self) -> Result<Option<PathBuf>, String> {
        Ok(self.stages()?.into_iter().next())
    }

    #[allow(dead_code)]
    pub(super) fn app_path(&self) -> &Path {
        &self.bundle
    }

    /// Restore only an unambiguous old backup when final app is missing.
    /// Keep other generations for inspection and never retry automatically.
    pub(super) fn recover(&self) -> Result<Recovery, String> {
        let stages = self.stages()?;
        let Some(stage) = stages.first() else {
            if !valid_directory(&self.bundle)? {
                return Err("installed application absent and no recoverable backup".into());
            }
            return Ok(Recovery::Clean);
        };
        let backup = stage.join(OLD_NAME);
        let prior = valid_directory(&backup)?;
        let current = valid_directory(&self.bundle)?;
        if prior && !current {
            fs::rename(&backup, &self.bundle)
                .map_err(|e| format!("cannot restore prior installed app: {e}"))?;
            let candidate = stage.join(APP_NAME);
            for item in fs::read_dir(stage).map_err(|e| e.to_string())? {
                let item = item.map_err(|e| e.to_string())?;
                if item.file_name() != OsStr::new(APP_NAME) {
                    return Err(format!(
                        "old app restored; unknown stage entries need manual review: {}",
                        stage.display()
                    ));
                }
            }
            if valid_directory(&candidate)? {
                fs::remove_dir_all(&candidate).map_err(|e| e.to_string())?;
            }
            fs::remove_dir(stage).map_err(|e| e.to_string())?;
            return Ok(Recovery::PreviousRestored);
        }
        Err(format!(
            "interrupted or ambiguous app update requires review: {}",
            stage.display()
        ))
    }

    pub(super) fn new_stage(&self, nonce: &str) -> Result<PathBuf, String> {
        if !(16..=64).contains(&nonce.len())
            || !nonce.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
        {
            return Err("invalid updater staging nonce".into());
        }
        if !self.stages()?.is_empty() {
            return Err("existing installer stage blocks the next update".into());
        }
        let stage = self.parent.join(format!("{STAGE_PREFIX}{nonce}"));
        fs::create_dir(&stage).map_err(|e| e.to_string())?;
        Ok(stage)
    }

    fn check_stage(&self, stage: &Path) -> Result<(), String> {
        if stage.parent() != Some(self.parent.as_path())
            || !stage.file_name().is_some_and(|name| name.to_string_lossy().starts_with(STAGE_PREFIX))
            || !valid_directory(stage)?
        {
            return Err("updater stage must be a real directory under Applications".into());
        }
        Ok(())
    }

    /// Two renames are not a filesystem transaction. A SIGKILL between them
    /// leaves a known backup, which recover() can restore on a future call.
    pub(super) fn promote(&self, stage: &Path) -> Result<(), String> {
        self.check_stage(stage)?;
        let fresh = stage.join(APP_NAME);
        let backup = stage.join(OLD_NAME);
        if !valid_directory(&fresh)? || !missing(&backup)? || !valid_directory(&self.bundle)? {
            return Err("new, old or backup app identities are inconsistent".into());
        }
        for required in ["Contents/Info.plist", "Contents/MacOS/aquilum-app"] {
            let path = fresh.join(required);
            match fs::symlink_metadata(path) {
                Ok(m) if m.file_type().is_file() && !m.file_type().is_symlink() => {}
                _ => return Err(format!("new bundle lacks regular file: {required}")),
            }
        }
        fs::rename(&self.bundle, &backup).map_err(|e| e.to_string())?;
        if let Err(err) = fs::rename(&fresh, &self.bundle) {
            if let Err(recovery) = fs::rename(&backup, &self.bundle) {
                return Err(format!(
                    "promotion failed ({err}); old app rollback failed ({recovery}); HOLD: {}",
                    stage.display()
                ));
            }
            return Err(format!("promotion failed; restored old app: {err}"));
        }
        Ok(())
    }

    /// A launch refusal restores the previous application without deleting
    /// the failed new bundle. Any failed restore keeps visible HOLD evidence.
    pub(super) fn rollback_launch(&self, stage: &Path) -> Result<(), String> {
        self.check_stage(stage)?;
        let backup = stage.join(OLD_NAME);
        let failed = stage.join("Aquilum.failed.app");
        if !valid_directory(&backup)? || !valid_directory(&self.bundle)? || !missing(&failed)? {
            return Err("ambiguous launch rollback; both app generations preserved".into());
        }
        fs::rename(&self.bundle, &failed).map_err(|e| e.to_string())?;
        fs::rename(&backup, &self.bundle)
            .map_err(|e| format!("could not restore old bundle; HOLD at {}: {e}", stage.display()))
    }

    /// A successful OS launch dispatch is NOT a new-binary health ACK.
    /// Deliberately retain old backup and stage for a later authenticated
    /// startup acknowledgement. Future updates encounter HOLD meanwhile.
    pub(super) fn preserve_until_startup_ack(&self, stage: &Path) -> Result<(), String> {
        self.check_stage(stage)?;
        if !valid_directory(&stage.join(OLD_NAME))? || !valid_directory(&self.bundle)? {
            return Err("missing old or new app during startup-acknowledgement HOLD".into());
        }
        Ok(())
    }

    /// A failed extraction, before any old->backup rename, may discard only
    /// the stage created by this run. Unknown sibling files are never deleted.
    pub(super) fn discard_unpublished(&self, stage: &Path) -> Result<(), String> {
        self.check_stage(stage)?;
        if !missing(&stage.join(OLD_NAME))? {
            return Err("refusing to discard stage holding a prior installed app".into());
        }
        let fresh = stage.join(APP_NAME);
        for item in fs::read_dir(stage).map_err(|e| e.to_string())? {
            let item = item.map_err(|e| e.to_string())?;
            if item.file_name() != OsStr::new(APP_NAME) {
                return Err("unknown staged payload; preserve for review".into());
            }
        }
        if valid_directory(&fresh)? {
            fs::remove_dir_all(&fresh).map_err(|e| e.to_string())?;
        }
        fs::remove_dir(stage).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
#[path = "macos_transaction/tests.rs"]
mod tests;
