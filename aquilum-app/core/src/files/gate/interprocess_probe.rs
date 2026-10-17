//! Test-only two-process OS-lock evidence for future bilateral File Gate.
//!
//! No normal Akula writes, sync mutations or recovery tools call this module.
//! Files are created only in private tempfile vaults. Path-based open is NOT
//! a trusted root-relative, no-follow OS capability.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

const TEST_LOCK_NAME: &str = ".akula-filegate-oslock-probe";
const CHILD_NAME: &str = "files::gate::interprocess_probe::tests::oslock_child_entrypoint";

struct HeldPair {
    // OS advisory locks are held until all owning handles are closed.
    _handles: Vec<File>,
}

fn try_one(root: &Path) -> io::Result<Option<File>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(root.join(TEST_LOCK_NAME))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

/// Ordering prevents cooperative two-vault deadlock but does NOT validate
/// root identity, hostile aliases or ownership. Strictly test-only.
fn try_both(left: &Path, right: &Path) -> io::Result<Option<HeldPair>> {
    let mut roots = [fs::canonicalize(left)?, fs::canonicalize(right)?];
    if roots[0] == roots[1] {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "two distinct probe vaults required",
        ));
    }
    roots.sort_unstable();
    let mut handles = Vec::with_capacity(2);
    for root in &roots {
        match try_one(root)? {
            Some(file) => handles.push(file),
            None => return Ok(None), // Drop any partially acquired handles.
        }
    }
    Ok(Some(HeldPair { _handles: handles }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command};
    use std::thread;
    use std::time::{Duration, Instant};

    // The compiled unit-test executable acts as a REAL second OS process.
    fn child_command(mode: &str, left: &Path, right: &Path, report: &Path) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .arg("--exact")
            .arg(CHILD_NAME)
            .arg("--nocapture")
            .env("AKULA_FILEGATE_OSLOCK_CHILD_MODE", mode)
            .env("AKULA_FILEGATE_OSLOCK_LEFT", left)
            .env("AKULA_FILEGATE_OSLOCK_RIGHT", right)
            .env("AKULA_FILEGATE_OSLOCK_REPORT", report);
        command
    }

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("source-vault");
        let second = temp.path().join("destination-vault");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        (temp, first, second)
    }

    fn try_in_other_process(left: &Path, right: &Path, report: &Path) -> String {
        let output = child_command("try", left, right, report).output().unwrap();
        assert!(
            output.status.success(),
            "child returned {:?}, stdout={:?}, stderr={:?}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        fs::read_to_string(report).unwrap()
    }

    struct KillChild(Child);

    impl KillChild {
        fn terminate(&mut self) {
            let _ = self.0.kill();
            self.0.wait().unwrap();
        }
    }

    impl Drop for KillChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn wait_for_report(child: &mut KillChild, report: &Path, expected: &[u8]) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if report.exists() {
                assert_eq!(fs::read(report).unwrap(), expected);
                return;
            }
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "child exited before reporting its lock state"
            );
            thread::sleep(Duration::from_millis(15));
        }
        panic!("child failed to report its lock state before deadline");
    }

    #[test]
    fn oslock_child_entrypoint() {
        let Some(mode) = std::env::var_os("AKULA_FILEGATE_OSLOCK_CHILD_MODE") else {
            return; // Normal suite without child environment.
        };
        let left = PathBuf::from(std::env::var_os("AKULA_FILEGATE_OSLOCK_LEFT").unwrap());
        let right = PathBuf::from(std::env::var_os("AKULA_FILEGATE_OSLOCK_RIGHT").unwrap());
        let report = PathBuf::from(std::env::var_os("AKULA_FILEGATE_OSLOCK_REPORT").unwrap());
        match mode.to_str().unwrap() {
            "try" => {
                let acquired = try_both(&left, &right).unwrap().is_some();
                fs::write(report, if acquired { "acquired" } else { "busy" }).unwrap();
            }
            "hold" => {
                let _pair = try_both(&left, &right).unwrap().expect("uncontended pair");
                fs::write(report, b"held").unwrap();
                loop {
                    thread::park_timeout(Duration::from_secs(60));
                }
            }
            "try-hold-after-decline" => {
                assert!(try_both(&left, &right).unwrap().is_none());
                fs::write(report, b"busy").unwrap();
                // Stay alive: parent must prove partial locks were dropped
                // before this process is killed or exits.
                loop {
                    thread::park_timeout(Duration::from_secs(60));
                }
            }
            other => panic!("unknown test-only child mode: {other}"),
        }
    }

    #[test]
    fn oslock_other_process_cannot_take_a_held_pair_in_reverse_order() {
        let (temp, left, right) = fixture();
        let held = try_both(&left, &right).unwrap().expect("first admission");
        let report = temp.path().join("reply.txt");
        assert_eq!(try_in_other_process(&right, &left, &report), "busy");
        drop(held);
        assert_eq!(try_in_other_process(&right, &left, &report), "acquired");
        for root in [&left, &right] {
            assert!(root.join(TEST_LOCK_NAME).is_file());
        }
    }

    #[test]
    fn oslock_partial_acquisition_is_released_when_second_vault_is_busy() {
        let (temp, left, right) = fixture();
        let mut ordered = [fs::canonicalize(left).unwrap(), fs::canonicalize(right).unwrap()];
        ordered.sort_unstable();
        let _second = try_one(&ordered[1]).unwrap().expect("lock second root");
        let report = temp.path().join("partial.txt");
        let child = child_command("try-hold-after-decline", &ordered[1], &ordered[0], &report)
            .spawn()
            .unwrap();
        let mut child = KillChild(child);
        wait_for_report(&mut child, &report, b"busy");
        // The child is STILL ALIVE. It obtained the first lock before
        // declining the second, so the first must already be released.
        assert!(child.0.try_wait().unwrap().is_none());
        assert!(try_one(&ordered[0]).unwrap().is_some());
        child.terminate();
    }

    #[test]
    fn oslock_killed_child_releases_both_handles_without_unlinking_lock_files() {
        let (temp, left, right) = fixture();
        let ready = temp.path().join("ready.txt");
        let child = child_command("hold", &left, &right, &ready).spawn().unwrap();
        let mut child = KillChild(child);
        wait_for_report(&mut child, &ready, b"held");
        assert!(try_both(&left, &right).unwrap().is_none());
        child.terminate();
        // Dead process releases OS file locks; no unlink means no split lock.
        assert!(try_both(&left, &right).unwrap().is_some());
        for root in [&left, &right] {
            assert!(root.join(TEST_LOCK_NAME).is_file());
        }
    }

    #[test]
    fn oslock_two_paths_resolving_to_same_vault_are_not_distinct_participants() {
        let (_temp, left, _right) = fixture();
        let alias = left.join(".");
        assert_eq!(try_both(&left, &alias).err().unwrap().kind(), ErrorKind::InvalidInput);
        assert!(!left.join(TEST_LOCK_NAME).exists());
    }
}
