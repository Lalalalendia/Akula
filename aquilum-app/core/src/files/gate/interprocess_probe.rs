//! Test-only two-process OS-lock evidence for future bilateral File Gate.
//!
//! No normal Akula writes, sync mutations or recovery tools call this module.
//! Files are created only in private tempfile vaults. Path-based open is NOT
//! a trusted root-relative, no-follow OS capability.

use crate::sync::filegate_lock_probe::{
    HeldProbeLock, PinnedProbeRoot, LOCK_NAME as TEST_LOCK_NAME,
};
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

const CHILD_NAME: &str = "files::gate::interprocess_probe::tests::oslock_child_entrypoint";

struct HeldPair {
    _handles: Vec<HeldProbeLock>,
}

fn try_one(root: &Path) -> io::Result<Option<HeldProbeLock>> {
    PinnedProbeRoot::open(root)?.try_lock()
}

/// Two workspace identities and their pre-existing lock objects are
/// inspected through pinned OS handles, never opened by ambient leaf paths.
/// Sorting prevents deadlock among cooperating processes. No positive
/// result authorizes a real File Gate mutation or distributed commit.
fn try_both(left: &Path, right: &Path) -> io::Result<Option<HeldPair>> {
    let mut roots = [
        (fs::canonicalize(left)?, PinnedProbeRoot::open(left)?),
        (fs::canonicalize(right)?, PinnedProbeRoot::open(right)?),
    ];
    if roots[0].1.identity() == roots[1].1.identity() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "two distinct pinned vault IDs required",
        ));
    }
    roots.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let mut handles = Vec::with_capacity(2);
    for (_, root) in &roots {
        match root.try_lock()? {
            Some(lock) => handles.push(lock),
            None => return Ok(None),
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
        // Bootstrap is intentionally isolated in the tempfile fixture.
        // The lock opener MUST fail if the file is missing at runtime.
        fs::write(first.join(TEST_LOCK_NAME), b"").unwrap();
        fs::write(second.join(TEST_LOCK_NAME), b"").unwrap();
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
            "hold-and-recheck-after-namespace-swap" => {
                // A real, independently running process keeps an OS lock on
                // the OLD inode/file ID while another process replaces the
                // pathname. A pinned descriptor is not a protected name.
                let root = PinnedProbeRoot::open(&left).unwrap();
                let held = root.try_lock().unwrap().expect("child holds original lock");
                fs::write(report.with_extension("ready"), b"held").unwrap();
                let proceed = report.with_extension("go");
                let deadline = Instant::now() + Duration::from_secs(20);
                while !proceed.exists() {
                    assert!(Instant::now() < deadline, "swap signal timed out");
                    thread::sleep(Duration::from_millis(10));
                }
                let refused = held.recheck_held_name(&root).is_err();
                fs::write(&report, if refused { "refused" } else { "unsafe" }).unwrap();
                // Stay ALIVE with old handle: the parent must observe that
                // both distinct lock objects can be held simultaneously.
                loop {
                    thread::park_timeout(Duration::from_secs(60));
                }
            }
            "bootstrap-on-signal" => {
                // Both child *processes* advertise readiness, then the parent
                // releases a shared barrier. Never create outside temp vaults.
                let start = PathBuf::from(
                    std::env::var_os("AKULA_FILEGATE_OSLOCK_START").unwrap()
                );
                fs::write(report.with_extension("ready"), b"ready").unwrap();
                let deadline = Instant::now() + Duration::from_secs(20);
                while !start.exists() {
                    assert!(Instant::now() < deadline, "bootstrap barrier timed out");
                    thread::sleep(Duration::from_millis(10));
                }
                let inspected = PinnedProbeRoot::open(&left).unwrap();
                let message = match inspected.bootstrap_exclusive_for_test() {
                    Ok(evidence) => format!(
                        "{}|{}|{}:{}",
                        if evidence.created { "created" } else { "existing" },
                        evidence.generation,
                        evidence.identity.0,
                        evidence.identity.1,
                    ),
                    // A second child can observe a still-incomplete winner's
                    // record: refusing is safe; overwriting is NOT.
                    Err(_) => "refused".to_string(),
                };
                fs::write(report, message).unwrap();
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
    fn oslock_missing_lock_object_fails_closed_without_recreation() {
        let (_temp, left, _right) = fixture();
        let lock = left.join(TEST_LOCK_NAME);
        fs::remove_file(&lock).unwrap();
        assert!(try_one(&left).is_err(), "missing lock must not get replaced");
        assert!(!lock.exists(), "test opener must not create a new lock");
    }

    #[test]
    fn oslock_hardlink_alias_is_rejected_even_if_both_names_refer_to_same_inode() {
        let (_temp, left, _right) = fixture();
        let lock = left.join(TEST_LOCK_NAME);
        let alias = left.join("second-hardlink-to-lock");
        fs::hard_link(&lock, &alias).unwrap();
        assert!(try_one(&left).is_err(), "linked lock object must be refused");
        assert!(alias.is_file(), "unsafe aliases must not be cleaned up blindly");
        assert!(lock.is_file());
    }

    #[test]
    fn oslock_pinned_root_replacement_is_detected_before_lock() {
        let (temp, left, _right) = fixture();
        let pinned = PinnedProbeRoot::open(&left).unwrap();
        let moved = temp.path().join("moved-original-root");
        fs::rename(&left, &moved).unwrap();
        fs::create_dir(&left).unwrap();
        fs::write(left.join(TEST_LOCK_NAME), b"replacement").unwrap();
        assert!(pinned.recheck_root().is_err());
        assert!(pinned.try_lock().is_err(), "root substitution must not get lock");
        assert_eq!(fs::read(left.join(TEST_LOCK_NAME)).unwrap(), b"replacement");
        assert!(moved.join(TEST_LOCK_NAME).is_file());
    }

    #[cfg(unix)]
    #[test]
    fn oslock_symlink_leaf_never_follows_a_different_file() {
        use std::os::unix::fs::symlink;
        let (temp, left, _right) = fixture();
        let lock = left.join(TEST_LOCK_NAME);
        let external = temp.path().join("outside-original");
        fs::rename(&lock, &external).unwrap();
        symlink(&external, &lock).unwrap();
        assert!(try_one(&left).is_err());
        assert_eq!(fs::read(&external).unwrap(), b"");
        assert!(fs::symlink_metadata(&lock).unwrap().file_type().is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn oslock_symlink_root_is_not_silently_canonicalized_to_vault() {
        use std::os::unix::fs::symlink;
        let (temp, left, _right) = fixture();
        let alias = temp.path().join("alias-root");
        symlink(&left, &alias).unwrap();
        assert!(try_one(&alias).is_err());
        assert!(try_one(&left).unwrap().is_some());
    }


    #[test]
    fn oslock_bootstrap_exclusive_create_then_reopen_keeps_generation_and_bytes() {
        let (_temp, left, _right) = fixture();
        let lock = left.join(TEST_LOCK_NAME);
        fs::remove_file(&lock).unwrap();
        let first_root = PinnedProbeRoot::open(&left).unwrap();
        let first = first_root.bootstrap_exclusive_for_test().unwrap();
        assert!(first.created);
        assert_ne!(first.generation, uuid::Uuid::nil());
        assert_ne!(first.identity.1, 0);
        #[cfg(windows)]
        assert!(!first.parent_sync_completed, "Windows parent fsync is not proven");
        let before = fs::read(&lock).unwrap();
        assert!(!before.is_empty());
        drop(first_root);

        // Simulate Core restart: a fresh handle sees the SAME file object,
        // not a recreated pathname and not a new local workspace UUID.
        let second_root = PinnedProbeRoot::open(&left).unwrap();
        let reopened = second_root.bootstrap_exclusive_for_test().unwrap();
        assert!(!reopened.created);
        assert_eq!(first.generation, reopened.generation);
        assert_eq!(first.identity, reopened.identity);
        assert_eq!(fs::read(&lock).unwrap(), before);
        assert!(second_root.try_lock().unwrap().is_some());
    }

    #[test]
    fn oslock_bootstrap_torn_record_is_never_repaired_or_overwritten() {
        let (_temp, left, _right) = fixture();
        let lock = left.join(TEST_LOCK_NAME);
        fs::write(&lock, b"interrupted-creator").unwrap();
        let pinned = PinnedProbeRoot::open(&left).unwrap();
        assert!(pinned.bootstrap_exclusive_for_test().is_err());
        assert_eq!(fs::read(&lock).unwrap(), b"interrupted-creator");

        fs::remove_file(&lock).unwrap();
        pinned.bootstrap_exclusive_for_test().unwrap();
        let mut damaged = fs::read(&lock).unwrap();
        *damaged.last_mut().unwrap() ^= 0x7a;
        fs::write(&lock, &damaged).unwrap();
        assert!(pinned.bootstrap_exclusive_for_test().is_err());
        assert_eq!(fs::read(&lock).unwrap(), damaged);
    }

    #[test]
    fn oslock_bootstrap_hardlink_and_swapped_root_refuse_without_cleanup() {
        let (temp, left, _right) = fixture();
        let lock = left.join(TEST_LOCK_NAME);
        fs::remove_file(&lock).unwrap();
        let pinned = PinnedProbeRoot::open(&left).unwrap();
        pinned.bootstrap_exclusive_for_test().unwrap();
        let saved = fs::read(&lock).unwrap();

        let alias = left.join("untrusted-hardlink");
        fs::hard_link(&lock, &alias).unwrap();
        assert!(pinned.bootstrap_exclusive_for_test().is_err());
        assert_eq!(fs::read(&alias).unwrap(), saved);
        fs::remove_file(alias).unwrap(); // Explicitly dispose test fixture alias.

        let moved = temp.path().join("original-vault-moved");
        fs::rename(&left, &moved).unwrap();
        fs::create_dir(&left).unwrap();
        assert!(pinned.bootstrap_exclusive_for_test().is_err());
        assert!(!left.join(TEST_LOCK_NAME).exists());
        assert_eq!(fs::read(moved.join(TEST_LOCK_NAME)).unwrap(), saved);
    }

    #[cfg(unix)]
    #[test]
    fn oslock_bootstrap_symlink_and_special_leaf_fail_closed() {
        use std::os::unix::fs::symlink;
        let (temp, left, _right) = fixture();
        let lock = left.join(TEST_LOCK_NAME);
        let outside = temp.path().join("outside-bytes");
        fs::write(&outside, b"never modified").unwrap();
        fs::remove_file(&lock).unwrap();
        symlink(&outside, &lock).unwrap();
        let pinned = PinnedProbeRoot::open(&left).unwrap();
        assert!(pinned.bootstrap_exclusive_for_test().is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"never modified");
        fs::remove_file(&lock).unwrap();
        fs::create_dir(&lock).unwrap();
        assert!(pinned.bootstrap_exclusive_for_test().is_err());
        assert!(lock.is_dir());
    }

    #[test]
    fn oslock_bootstrap_two_real_processes_never_create_two_lock_generations() {
        let (temp, left, right) = fixture();
        fs::remove_file(left.join(TEST_LOCK_NAME)).unwrap();
        let start = temp.path().join("bootstrap-start.signal");
        let reports = [
            temp.path().join("bootstrap-child-a.txt"),
            temp.path().join("bootstrap-child-b.txt"),
        ];
        let mut children: Vec<KillChild> = reports.iter().map(|report| {
            KillChild(
                child_command("bootstrap-on-signal", &left, &right, report)
                    .env("AKULA_FILEGATE_OSLOCK_START", &start)
                    .spawn()
                    .unwrap()
            )
        }).collect();
        for (child, report) in children.iter_mut().zip(reports.iter()) {
            wait_for_report(child, &report.with_extension("ready"), b"ready");
        }
        // The child processes are independently alive at the release point.
        assert!(children.iter_mut().all(|child| child.0.try_wait().unwrap().is_none()));
        fs::write(&start, b"start").unwrap();
        for child in &mut children {
            assert!(child.0.wait().unwrap().success());
        }
        let results: Vec<String> = reports.iter()
            .map(|report| fs::read_to_string(report).unwrap())
            .collect();
        let creators: Vec<&String> = results.iter()
            .filter(|result| result.starts_with("created|"))
            .collect();
        assert_eq!(creators.len(), 1, "exclusive create must have one winner: {results:?}");

        let creator_tokens: Vec<&str> = creators[0].split('|').collect();
        assert_eq!(creator_tokens.len(), 3);
        let remaining = results.iter().find(|result| !result.starts_with("created|")).unwrap();
        assert!(remaining == "refused" || remaining.starts_with("existing|"), "{results:?}");
        if remaining.starts_with("existing|") {
            let read_tokens: Vec<&str> = remaining.split('|').collect();
            assert_eq!(read_tokens[1..], creator_tokens[1..]);
        }
        let third = PinnedProbeRoot::open(&left).unwrap()
            .bootstrap_exclusive_for_test().unwrap();
        assert!(!third.created);
        assert_eq!(third.generation.to_string(), creator_tokens[1]);
        assert_eq!(format!("{}:{}", third.identity.0, third.identity.1), creator_tokens[2]);
        assert!(left.join(TEST_LOCK_NAME).is_file());
    }

    fn demonstrate_live_split_lock_detected(clone_valid_record: bool) {
        let (temp, left, right) = fixture();
        let original = left.join(TEST_LOCK_NAME);
        fs::remove_file(&original).unwrap();
        let seed = PinnedProbeRoot::open(&left).unwrap();
        let expected = seed.bootstrap_exclusive_for_test().unwrap();
        assert!(expected.created);
        let original_bytes = fs::read(&original).unwrap();

        let report = temp.path().join("held-process-report.txt");
        let child = child_command("hold-and-recheck-after-namespace-swap", &left, &right, &report)
            .spawn()
            .unwrap();
        let mut child = KillChild(child);
        wait_for_report(&mut child, &report.with_extension("ready"), b"held");
        assert!(child.0.try_wait().unwrap().is_none());

        // The OLD lock remains open (and exclusively locked) in the child,
        // but a privileged/hostile namespace writer can rename its name.
        // Neither File::try_lock nor a BLAKE3 checksum prevents this.
        let orphaned = temp.path().join("old-held-lock-renamed-outside-vault");
        fs::rename(&original, &orphaned).unwrap();
        if clone_valid_record {
            // Forge a BYTE-IDENTICAL record with the SAME generation UUID:
            // only physical file identity can distinguish the objects.
            fs::write(&original, &original_bytes).unwrap();
        } else {
            let fresh = PinnedProbeRoot::open(&left).unwrap();
            let replacement = fresh.bootstrap_exclusive_for_test().unwrap();
            assert!(replacement.created);
            assert_ne!(expected.generation, replacement.generation);
            assert_ne!(expected.identity, replacement.identity);
        }
        let new_bytes = fs::read(&original).unwrap();
        assert_eq!(fs::read(&orphaned).unwrap(), original_bytes);
        let fresh = PinnedProbeRoot::open(&left).unwrap();
        // THIS SUCCESS IS EVIDENCE OF A SPLIT LOCK, NOT A SAFE ADMISSION.
        // A new file ID has a separate OS lock although the child is alive.
        let new_lock = fresh.try_lock().unwrap().expect(
            "a replaced pathname can be locked despite a live old holder"
        );
        assert_ne!(new_lock.identity(), expected.identity);
        assert!(child.0.try_wait().unwrap().is_none());

        fs::write(report.with_extension("go"), b"go").unwrap();
        wait_for_report(&mut child, &report, b"refused");
        assert!(child.0.try_wait().unwrap().is_none());
        assert_eq!(fs::read(&orphaned).unwrap(), original_bytes);
        assert_eq!(fs::read(&original).unwrap(), new_bytes);
        drop(new_lock);
        child.terminate();
    }

    #[test]
    fn oslock_live_split_lock_after_rename_and_rebootstrap_is_detected() {
        demonstrate_live_split_lock_detected(false);
    }

    #[test]
    fn oslock_live_split_lock_with_cloned_valid_generation_is_detected() {
        demonstrate_live_split_lock_detected(true);
    }

    #[test]
    fn oslock_two_paths_resolving_to_same_vault_are_not_distinct_participants() {
        let (_temp, left, _right) = fixture();
        let alias = left.join(".");
        assert_eq!(try_both(&left, &alias).err().unwrap().kind(), ErrorKind::InvalidInput);
        assert!(left.join(TEST_LOCK_NAME).is_file());
    }
}
