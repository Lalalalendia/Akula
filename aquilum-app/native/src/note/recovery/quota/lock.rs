//! Persistent cross-process quota lock; never unlink/replace the lock file.
mod open;
mod nonblocking;
pub(in crate::note::recovery) use nonblocking::try_acquire;

use std::{
    fs::{self, File, TryLockError},
    io,
    path::Path,
    thread,
    time::{Duration, Instant},
};

use super::super::{denied, reparse};

pub(super) const FILENAME: &str = ".quota.lock";
const RETRY: Duration = Duration::from_millis(25);
const MAX_WAIT: Duration = Duration::from_secs(5);

pub(super) fn validate(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || reparse(&meta) || meta.len() != 0 {
        return Err(denied("unsafe recovery quota lock file"));
    }
    Ok(())
}

/// The File retains the lock through quota admission, sync and publication.
/// A timeout fails closed instead of blocking the disk worker forever.
pub(in crate::note::recovery) fn acquire(bucket: &Path) -> io::Result<File> {
    let file = open::validated(bucket)?;
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) if started.elapsed() < MAX_WAIT => {
                thread::sleep(RETRY);
            }
            Err(TryLockError::WouldBlock) => {
                return Err(denied("recovery quota busy; retry the shadow snapshot"));
            }
            Err(TryLockError::Error(error)) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::OpenOptions, path::PathBuf, process::Command};

    fn bucket(root: &Path) -> PathBuf {
        let bucket = root.join("akula-note-recovery-v1").join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&bucket).unwrap();
        bucket
    }

    #[test]
    fn quota_lock_probe_child() {
        let Ok(dir) = std::env::var("AKULA_QUOTA_LOCK_CHILD_DIR") else { return };
        let root = PathBuf::from(dir);
        let bucket = PathBuf::from(std::env::var("AKULA_QUOTA_LOCK_CHILD_BUCKET").unwrap());
        let guard = OpenOptions::new().read(true).write(true)
            .open(bucket.parent().unwrap().join(FILENAME)).unwrap();
        match guard.try_lock() {
            Err(TryLockError::WouldBlock) => fs::write(root.join("blocked"), "yes").unwrap(),
            _ => panic!("second process must observe the parent's live exclusive lock"),
        }
        let _guard = acquire(&bucket).unwrap();
        fs::write(root.join("acquired"), "locked").unwrap();
    }

    #[test]
    fn different_process_waits_for_quota_lock_release() {
        let temp = tempfile::tempdir().unwrap();
        let bucket = bucket(temp.path());
        let guard = acquire(&bucket).unwrap();
        let root = temp.path();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("quota_lock_probe_child")
            .arg("--nocapture")
            .env("AKULA_QUOTA_LOCK_CHILD_DIR", root)
            .env("AKULA_QUOTA_LOCK_CHILD_BUCKET", &bucket)
            .spawn().unwrap();
        let start = Instant::now();
        while !root.join("blocked").exists() && start.elapsed() < Duration::from_secs(10) {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(root.join("blocked").exists(), "child failed to observe lock contention");
        assert!(!root.join("acquired").exists(), "child bypassed live quota lock");
        drop(guard);
        assert!(child.wait().unwrap().success(), "child failed to acquire lock after release");
        assert_eq!(fs::read(root.join("acquired")).unwrap(), b"locked");
        assert_eq!(fs::metadata(bucket.parent().unwrap().join(FILENAME)).unwrap().len(), 0);
    }

    #[test]
    fn unsafe_quota_lock_refuses_admission_without_deleting_file() {
        let temp = tempfile::tempdir().unwrap();
        let bucket = bucket(temp.path());
        let path = bucket.parent().unwrap().join(FILENAME);
        fs::write(&path, "foreign data").unwrap();
        assert!(acquire(&bucket).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"foreign data");
    }

    #[test]
    fn legitimate_root_lock_is_not_counted_as_recovery_workpace() {
        let temp = tempfile::tempdir().unwrap();
        let bucket = bucket(temp.path());
        let _guard = acquire(&bucket).unwrap();
        assert!(super::super::admit(&bucket, 1024).is_ok());
    }
}
