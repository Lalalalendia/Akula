//! UI rename must never wait for a live cross-process snapshot writer.
use std::{fs::{File, TryLockError}, io, path::Path};

use super::super::super::denied;

pub(in crate::note::recovery) fn try_acquire(bucket: &Path) -> io::Result<File> {
    let file = super::open::validated(bucket)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => {
            Err(denied("recovery writer busy; retry the rename"))
        }
        Err(TryLockError::Error(error)) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::{Duration, Instant}};

    #[test]
    fn ui_move_lease_refuses_live_snapshot_writer_without_five_second_wait() {
        let temp = tempfile::tempdir().unwrap();
        let bucket = temp.path().join("akula-note-recovery-v1")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&bucket).unwrap();
        let writer = super::super::acquire(&bucket).unwrap();
        let started = Instant::now();
        assert!(try_acquire(&bucket).is_err(), "busy writer must block rename");
        assert!(started.elapsed() < Duration::from_secs(3),
            "UI lock attempt must not wait five seconds");
        drop(writer);
        assert!(try_acquire(&bucket).is_ok(), "released writer allows retry");
    }
}
