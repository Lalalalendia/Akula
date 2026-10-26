use super::{hash_bytes, write_file_atomic_impl};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};
use std::thread;
use std::time::{Duration, Instant};

const WRITER_CHILD: &str =
    "files::document::atomic_kill_tests::atomic_write_kill_child_entrypoint";
const RESTART_CHILD: &str =
    "files::document::atomic_kill_tests::atomic_write_restart_verifier_entrypoint";
const OLD_BYTES: &[u8] = b"previous complete revision";
const NEW_LEN: usize = 1024 * 1024;

/// Test-only pause seam for hard-terminating the real atomic publisher at
/// filesystem phase boundaries. It can only park on a note inside an exact
/// disposable fixture; production builds contain neither this function nor
/// any environment-controlled writer behavior.
pub(super) fn atomic_write_kill_probe(path: &Path, stage: &str) {
    if std::env::var("AKULA_ATOMIC_KILL_STAGE").as_deref() != Ok(stage) {
        return;
    }
    let workdir = std::env::var_os("AKULA_ATOMIC_KILL_WORKDIR")
        .expect("atomic kill probe requires a disposable workdir");
    let workdir = Path::new(&workdir);
    let canonical_workdir = crate::search::paths::canonical_path(workdir);
    let canonical_target = crate::search::paths::canonical_path(path);
    let relative = crate::search::paths::strip_root(&canonical_workdir, &canonical_target)
        .expect("atomic kill probe target must stay inside its disposable workdir");
    let mut components = relative.components();
    let fixture = components.next().expect("atomic kill fixture directory");
    let target = components.next().expect("atomic kill target filename");
    assert!(
        components.next().is_none(),
        "atomic kill probe target must be one file"
    );
    assert!(
        fixture
            .as_os_str()
            .to_string_lossy()
            .starts_with("atomic-write-crash-fixture-"),
        "atomic kill probe target must be inside its named fixture"
    );
    assert_eq!(target.as_os_str(), std::ffi::OsStr::new("Reserved.md"));

    std::fs::write(workdir.join("atomic-write-kill-marker"), stage.as_bytes())
        .expect("write test-only atomic phase marker");
    loop {
        std::thread::park_timeout(std::time::Duration::from_millis(100));
    }
}

fn launch(test_name: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("libtest executable"));
    command.arg("--exact").arg(test_name).arg("--nocapture");
    command
}

fn wait_for_marker(child: &mut Child, marker: &Path, stage: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if marker.exists() {
            assert_eq!(fs::read(marker).unwrap(), stage.as_bytes());
            return;
        }
        if let Some(status) = child.try_wait().expect("poll writer child") {
            panic!("writer exited before phase {stage}: {status}");
        }
        assert!(
            Instant::now() < deadline,
            "writer did not reach phase {stage}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn temp_files_for(path: &Path) -> Vec<PathBuf> {
    let prefix = format!(".{}.", path.file_name().unwrap().to_string_lossy());
    fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|candidate| {
            candidate
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&prefix)
        })
        .collect()
}

#[test]
fn atomic_write_kill_child_entrypoint() {
    let (Some(target), Some(stage), Some(workdir), Some(candidate)) = (
        std::env::var_os("AKULA_ATOMIC_KILL_TARGET"),
        std::env::var_os("AKULA_ATOMIC_KILL_STAGE"),
        std::env::var_os("AKULA_ATOMIC_KILL_WORKDIR"),
        std::env::var_os("AKULA_ATOMIC_KILL_CANDIDATE"),
    ) else {
        return;
    };
    let target = PathBuf::from(target);
    let stage = stage.to_string_lossy().into_owned();
    let workdir = PathBuf::from(workdir);
    let candidate = fs::read_to_string(candidate).expect("candidate fixture text");
    let old_hash = hash_bytes(OLD_BYTES);
    assert!(matches!(
        stage.as_str(),
        "after-temp-open"
            | "after-first-half"
            | "after-temp-write-before-cas"
            | "after-cas-before-commit"
            | "after-commit"
    ));
    assert_eq!(fs::read(&target).unwrap(), OLD_BYTES);

    let result = write_file_atomic_impl(&target, &candidate, Some(&old_hash));
    panic!(
        "writer returned before parent killed it at {stage}: {result:?}; workdir={}",
        workdir.display()
    );
}

#[test]
fn atomic_write_restart_verifier_entrypoint() {
    let (Some(target), Some(expected_path), Some(variant)) = (
        std::env::var_os("AKULA_ATOMIC_RESTART_TARGET"),
        std::env::var_os("AKULA_ATOMIC_RESTART_EXPECTED"),
        std::env::var_os("AKULA_ATOMIC_RESTART_VARIANT"),
    ) else {
        return;
    };
    let target = PathBuf::from(target);
    let expected = fs::read(expected_path).expect("restart expectation fixture");
    let actual = fs::read(&target).expect("published note after child termination");
    assert_eq!(
        actual, expected,
        "restart must expose one complete generation"
    );
    println!(
        "AKULA_ATOMIC_RESTART_VERIFY variant={} bytes={} blake3={}",
        variant.to_string_lossy(),
        actual.len(),
        hash_bytes(&actual)
    );
}

#[test]
fn forced_process_kill_at_atomic_file_phases_keeps_old_or_complete_new_bytes() {
    let candidate = "N".repeat(NEW_LEN);
    let split = candidate.len() / 2;
    let new_hash = hash_bytes(candidate.as_bytes());
    let cases = [
        ("after-temp-open", "old", 0usize),
        ("after-first-half", "old", split),
        ("after-temp-write-before-cas", "old", NEW_LEN),
        ("after-cas-before-commit", "old", NEW_LEN),
        ("after-commit", "new", 0usize),
    ];

    for (stage, visible, staged_len) in cases {
        let temp = tempfile::tempdir().expect("isolated atomic-write parent");
        let fixture = temp
            .path()
            .join(format!("atomic-write-crash-fixture-{stage}"));
        fs::create_dir(&fixture).unwrap();
        let target = fixture.join("Reserved.md");
        let candidate_path = fixture.join("candidate.txt");
        let old_path = fixture.join("old.expected");
        let new_path = fixture.join("new.expected");
        fs::write(&target, OLD_BYTES).unwrap();
        fs::write(&candidate_path, candidate.as_bytes()).unwrap();
        fs::write(&old_path, OLD_BYTES).unwrap();
        fs::write(&new_path, candidate.as_bytes()).unwrap();

        let marker = temp.path().join("atomic-write-kill-marker");
        let mut child = launch(WRITER_CHILD)
            .env("AKULA_ATOMIC_KILL_STAGE", stage)
            .env("AKULA_ATOMIC_KILL_WORKDIR", temp.path())
            .env("AKULA_ATOMIC_KILL_TARGET", &target)
            .env("AKULA_ATOMIC_KILL_CANDIDATE", &candidate_path)
            .spawn()
            .expect("start real atomic publisher in isolated child process");
        wait_for_marker(&mut child, &marker, stage);

        // Child::kill uses TerminateProcess on Windows and SIGKILL on Unix;
        // no Rust destructor can discard the deliberately staged temp file.
        child.kill().expect("forcibly terminate the paused writer");
        let killed = child.wait().expect("reap forcibly terminated writer");
        assert!(
            !killed.success(),
            "writer must not return after hard kill at {stage}"
        );

        let actual = fs::read(&target).expect("primary target remains readable");
        let expected = if visible == "new" {
            candidate.as_bytes()
        } else {
            OLD_BYTES
        };
        assert_eq!(actual, expected, "primary path at {stage}");
        let expected_hash = if visible == "new" {
            new_hash.clone()
        } else {
            hash_bytes(OLD_BYTES)
        };
        assert_eq!(hash_bytes(&actual), expected_hash);

        let temp_files = temp_files_for(&target);
        if stage == "after-commit" {
            assert!(
                temp_files.is_empty(),
                "committed rename must remove the staged name"
            );
        } else {
            assert_eq!(
                temp_files.len(),
                1,
                "abruptly killed pre-commit writer leaves its temp file"
            );
            assert_eq!(
                fs::read(&temp_files[0]).unwrap(),
                candidate.as_bytes()[..staged_len],
                "staged temp bytes at {stage}"
            );
        }

        let expected_path = if visible == "new" {
            &new_path
        } else {
            &old_path
        };
        let output: Output = launch(RESTART_CHILD)
            .env("AKULA_ATOMIC_RESTART_TARGET", &target)
            .env("AKULA_ATOMIC_RESTART_EXPECTED", expected_path)
            .env("AKULA_ATOMIC_RESTART_VARIANT", visible)
            .output()
            .expect("launch independent cold file verifier");
        assert!(
            output.status.success(),
            "cold verifier failed at {stage}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("AKULA_ATOMIC_RESTART_VERIFY"),
            "missing verifier evidence: {stdout}"
        );
        println!(
            "PC_FG_02_ATOMIC_KILL stage={stage} child_killed=1 primary={visible} \
            primary_bytes={} primary_blake3={} temp_files={} temp_bytes={} restart=PASS",
            actual.len(),
            hash_bytes(&actual),
            temp_files.len(),
            staged_len
        );
        print!("{stdout}");
    }
}

#[test]
fn stale_cas_after_staging_preserves_external_bytes_and_discards_temp() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = temp.path().join("atomic-write-crash-fixture-stale-cas");
    fs::create_dir(&fixture).unwrap();
    let target = fixture.join("Reserved.md");
    fs::write(&target, b"external revision").unwrap();

    let error = write_file_atomic_impl(&target, "candidate revision", Some("stale hash"))
        .expect_err("stale CAS must reject the complete staged candidate");

    assert!(matches!(
        error,
        crate::files::error::FileCommandError::Conflict { .. }
    ));
    assert_eq!(fs::read(&target).unwrap(), b"external revision");
    assert!(
        temp_files_for(&target).is_empty(),
        "ordinary CAS failure should discard its temp file"
    );
}
