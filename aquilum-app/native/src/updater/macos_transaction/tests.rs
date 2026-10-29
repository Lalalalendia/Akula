use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    home: PathBuf,
    parent: PathBuf,
    app: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let home = std::env::temp_dir()
            .join(format!("akula-updater-txn-{}-{id}", std::process::id()));
        let parent = home.join("Applications");
        let app = parent.join(APP_NAME);
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        fs::write(app.join("Contents/Info.plist"), b"old-info").unwrap();
        fs::write(app.join("Contents/MacOS/aquilum-app"), b"old-binary").unwrap();
        Self { home, parent, app }
    }

    fn lock(&self) -> AppGate {
        AppGate::acquire(&self.app, &self.home).unwrap()
    }

    fn stage_with_new(&self, gate: &AppGate) -> PathBuf {
        let stage = gate.new_stage("aabbccddeeff00112233445566778899").unwrap();
        let fresh = stage.join(APP_NAME);
        fs::create_dir_all(fresh.join("Contents/MacOS")).unwrap();
        fs::write(fresh.join("Contents/Info.plist"), b"new-info").unwrap();
        fs::write(fresh.join("Contents/MacOS/aquilum-app"), b"new-binary").unwrap();
        stage
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}

#[test]
fn fresh_user_install_has_one_shared_lock() {
    let fixture = Fixture::new();
    let first = fixture.lock();
    assert_eq!(first.recover().unwrap(), Recovery::Clean);
    let blocked = AppGate::acquire(&fixture.app, &fixture.home);
    assert!(blocked.is_err(), "a second independent fd bypassed BSD flock");
    drop(first);
    let reacquired = AppGate::acquire(&fixture.app, &fixture.home);
    assert!(
        reacquired.is_ok(),
        "BSD lock must be released on drop, but reacquisition failed: {:?}",
        reacquired.err()
    );
}

#[test]
fn staging_is_unique_and_never_deletes_a_prior_run() {
    let fixture = Fixture::new();
    let gate = fixture.lock();
    let stage = gate.new_stage("aabbccddeeff00112233445566778899").unwrap();
    fs::write(stage.join("unrelated"), b"preserve").unwrap();
    assert!(gate.new_stage("aabbccddeeff00112233445566778899").is_err());
    assert_eq!(fs::read(stage.join("unrelated")).unwrap(), b"preserve");
    assert!(gate.recover().unwrap_err().contains("requires review"));
}

#[test]
fn interrupted_old_to_backup_restores_before_any_update() {
    let fixture = Fixture::new();
    let gate = fixture.lock();
    let stage = fixture.stage_with_new(&gate);
    fs::rename(&fixture.app, stage.join(OLD_NAME)).unwrap();
    assert!(!fixture.app.exists());
    assert_eq!(gate.recover().unwrap(), Recovery::PreviousRestored);
    assert_eq!(fs::read(fixture.app.join("Contents/Info.plist")).unwrap(), b"old-info");
    assert!(!stage.exists());
}

#[test]
fn missing_app_is_allowed_into_recovery_under_same_lock() {
    let fixture = Fixture::new();
    let stage;
    {
        let gate = fixture.lock();
        stage = fixture.stage_with_new(&gate);
        fs::rename(&fixture.app, stage.join(OLD_NAME)).unwrap();
    }
    let next = fixture.lock();
    assert_eq!(next.recover().unwrap(), Recovery::PreviousRestored);
    assert_eq!(fs::read(fixture.app.join("Contents/Info.plist")).unwrap(), b"old-info");
}

#[test]
fn interrupted_after_new_publication_preserves_both_generations() {
    let fixture = Fixture::new();
    let gate = fixture.lock();
    let stage = fixture.stage_with_new(&gate);
    gate.promote(&stage).unwrap();
    assert!(gate.recover().unwrap_err().contains("requires review"));
    assert_eq!(fs::read(fixture.app.join("Contents/Info.plist")).unwrap(), b"new-info");
    assert_eq!(fs::read(stage.join(OLD_NAME).join("Contents/Info.plist")).unwrap(), b"old-info");
}

#[test]
fn successful_launch_dispatch_does_not_destroy_old_backup() {
    let fixture = Fixture::new();
    let gate = fixture.lock();
    let stage = fixture.stage_with_new(&gate);
    gate.promote(&stage).unwrap();
    gate.preserve_until_startup_ack(&stage).unwrap();
    assert!(stage.join(OLD_NAME).exists());
    assert!(gate.recover().is_err(), "new process has not acknowledged its own startup");
}

#[test]
fn launch_failure_restores_old_without_deleting_new_generation() {
    let fixture = Fixture::new();
    let gate = fixture.lock();
    let stage = fixture.stage_with_new(&gate);
    gate.promote(&stage).unwrap();
    gate.rollback_launch(&stage).unwrap();
    assert_eq!(fs::read(fixture.app.join("Contents/MacOS/aquilum-app")).unwrap(), b"old-binary");
    assert_eq!(
        fs::read(stage.join("Aquilum.failed.app/Contents/MacOS/aquilum-app")).unwrap(),
        b"new-binary"
    );
    assert!(gate.recover().is_err());
}

#[test]
fn multiple_abandoned_stages_refuse_arbitrary_rollback_choice() {
    let fixture = Fixture::new();
    let gate = fixture.lock();
    let _first = fixture.stage_with_new(&gate);
    fs::create_dir(fixture.parent.join(".akula-install.ffffffffffffffff")).unwrap();
    assert!(gate.recover().unwrap_err().contains("multiple"));
}

#[test]
fn recovery_restores_old_but_preserves_unknown_staging_payload() {
    let fixture = Fixture::new();
    let gate = fixture.lock();
    let stage = fixture.stage_with_new(&gate);
    fs::rename(&fixture.app, stage.join(OLD_NAME)).unwrap();
    fs::write(stage.join("unknown.txt"), b"preserve").unwrap();
    assert!(gate.recover().unwrap_err().contains("unknown stage"));
    assert_eq!(fs::read(stage.join("unknown.txt")).unwrap(), b"preserve");
    assert_eq!(fs::read(fixture.app.join("Contents/Info.plist")).unwrap(), b"old-info");
}

#[test]
fn failed_unpack_only_discards_known_unpublished_candidate() {
    let fixture = Fixture::new();
    let gate = fixture.lock();
    let stage = fixture.stage_with_new(&gate);
    gate.discard_unpublished(&stage).unwrap();
    assert!(!stage.exists());
    assert_eq!(fs::read(fixture.app.join("Contents/Info.plist")).unwrap(), b"old-info");
}

#[test]
fn unknown_prepared_files_are_not_recursively_destroyed() {
    let fixture = Fixture::new();
    let gate = fixture.lock();
    let stage = fixture.stage_with_new(&gate);
    fs::write(stage.join("unrecognized.txt"), b"important").unwrap();
    assert!(gate.discard_unpublished(&stage).is_err());
    assert_eq!(fs::read(stage.join("unrecognized.txt")).unwrap(), b"important");
}

#[test]
fn cannot_discard_any_previous_app_backup() {
    let fixture = Fixture::new();
    let gate = fixture.lock();
    let stage = fixture.stage_with_new(&gate);
    fs::rename(&fixture.app, stage.join(OLD_NAME)).unwrap();
    assert!(gate.discard_unpublished(&stage).unwrap_err().contains("prior installed"));
    assert!(stage.join(OLD_NAME).exists());
}

#[test]
fn refuses_alternate_installation_authority() {
    let fixture = Fixture::new();
    let other = fixture.home.join("Different");
    fs::create_dir(&other).unwrap();
    let other_app = other.join(APP_NAME);
    fs::create_dir(&other_app).unwrap();
    assert!(AppGate::acquire(&other_app, &fixture.home)
        .unwrap_err()
        .contains("outside the shared"));
}

#[test]
fn refuses_a_symlinked_lock_file() {
    let fixture = Fixture::new();
    let lock = fixture.parent.join(LOCK_NAME);
    #[cfg(unix)]
    std::os::unix::fs::symlink(fixture.app.join("Contents/Info.plist"), lock).unwrap();
    assert!(AppGate::acquire(&fixture.app, &fixture.home).is_err());
}

#[test]
fn rejects_symlinked_replacement_bundle() {
    let fixture = Fixture::new();
    fs::rename(&fixture.app, fixture.parent.join("Real.app")).unwrap();
    std::os::unix::fs::symlink(fixture.parent.join("Real.app"), &fixture.app).unwrap();
    assert!(AppGate::acquire(&fixture.app, &fixture.home).is_err());
}

#[test]
fn parser_output_must_contain_both_regular_essential_files() {
    let fixture = Fixture::new();
    let gate = fixture.lock();
    let stage = gate.new_stage("0123456789abcdef0123456789abcdef").unwrap();
    fs::create_dir(stage.join(APP_NAME)).unwrap();
    assert!(gate.promote(&stage).unwrap_err().contains("lacks regular file"));
    assert_eq!(fs::read(fixture.app.join("Contents/Info.plist")).unwrap(), b"old-info");
}

#[test]
fn child_crash_at_transaction_boundary() {
    let Ok(phase) = std::env::var("AKULA_TEST_CRASH_PHASE") else {
        return; // This is a subprocess helper, not a normal fixture.
    };
    let home = PathBuf::from(std::env::var_os("AKULA_TEST_CRASH_HOME").unwrap());
    let marker = PathBuf::from(std::env::var_os("AKULA_TEST_CRASH_MARKER").unwrap());
    let app = home.join("Applications").join(APP_NAME);
    let gate = AppGate::acquire(&app, &home).unwrap();
    let stage = gate.new_stage("abcdef0123456789abcdef0123456789").unwrap();
    let fresh = stage.join(APP_NAME);
    fs::create_dir_all(fresh.join("Contents/MacOS")).unwrap();
    fs::write(fresh.join("Contents/Info.plist"), b"new-info").unwrap();
    fs::write(fresh.join("Contents/MacOS/aquilum-app"), b"new-binary").unwrap();
    match phase.as_str() {
        "after-backup" => fs::rename(&app, stage.join(OLD_NAME)).unwrap(),
        "after-promotion" => gate.promote(&stage).unwrap(),
        _ => panic!("unknown test crash phase"),
    }
    fs::write(marker, b"ready to kill").unwrap();
    std::thread::sleep(std::time::Duration::from_secs(60));
    panic!("subprocess was not terminated at transaction boundary");
}

fn kill_child_at(phase: &str) -> Fixture {
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::Duration;
    let fixture = Fixture::new();
    let marker = fixture.home.join("crash-marker");
    // In standalone rustc --test and cargo test, the test harness may
    // qualify module names differently. Discover the actual --exact name
    // instead of assuming module_path! matches the harness name.
    let executable = std::env::current_exe().unwrap();
    let listing = Command::new(&executable).arg("--list").output().unwrap();
    assert!(listing.status.success(), "cannot list subprocess test names");
    let listed = String::from_utf8_lossy(&listing.stdout);
    // Resolve the helper by this exact module's name. Both Linux and
    // macOS transaction tests contain a child_crash_at_transaction_boundary
    // helper; a suffix-only lookup can silently launch the other helper
    // with the wrong environment and report a false crash-test failure.
    // The printed test list omits the binary crate name, while
    // module_path!() includes it.
    let suite = module_path!()
        .split_once("::")
        .map_or(module_path!(), |(_, rest)| rest);
    let exact = format!("{suite}::child_crash_at_transaction_boundary");
    let helper = listed
        .lines()
        .filter_map(|line| line.strip_suffix(": test"))
        .find(|name| *name == exact.as_str())
        .expect("test harness did not register this module's crash subprocess helper");
    let mut child = Command::new(&executable)
        .arg("--exact")
        .arg(helper)
        .arg("--nocapture")
        .env("AKULA_TEST_CRASH_PHASE", phase)
        .env("AKULA_TEST_CRASH_HOME", &fixture.home)
        .env("AKULA_TEST_CRASH_MARKER", &marker)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut reached = false;
    for _ in 0..200 {
        if marker.is_file() {
            reached = true;
            break;
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(reached, "child never reached the target crash window");
    fixture
}

#[test]
fn actual_killed_child_after_backup_releases_lock_and_restores_old_app() {
    let fixture = kill_child_at("after-backup");
    let guard = fixture.lock();
    assert_eq!(guard.recover().unwrap(), Recovery::PreviousRestored);
    assert_eq!(fs::read(fixture.app.join("Contents/Info.plist")).unwrap(), b"old-info");
}

#[test]
fn actual_killed_child_after_new_publication_preserves_both_versions() {
    let fixture = kill_child_at("after-promotion");
    let guard = fixture.lock();
    assert!(guard.recover().unwrap_err().contains("requires review"));
    assert_eq!(fs::read(fixture.app.join("Contents/Info.plist")).unwrap(), b"new-info");
    let stage = fixture.parent.join(".akula-install.abcdef0123456789abcdef0123456789");
    assert_eq!(fs::read(stage.join(OLD_NAME).join("Contents/Info.plist")).unwrap(), b"old-info");
}

#[cfg(target_os = "macos")]
#[test]
fn native_bsd_lockf_refuses_same_lock_held_from_rust() {
    use std::process::Command;
    let fixture = Fixture::new();
    let _held = fixture.lock();
    let status = Command::new("/usr/bin/lockf")
        .arg("-k")
        .arg("-t")
        .arg("0")
        .arg(fixture.parent.join(LOCK_NAME))
        .arg("/usr/bin/true")
        .status()
        .unwrap();
    assert!(!status.success(), "macOS lockf must see the Rust-held BSD flock");
}

#[test]
fn guard_drop_unlocks_an_inherited_duplicate_fd() {
    let fixture = Fixture::new();
    let gate = fixture.lock();
    let inherited_fd = gate._lock.try_clone().unwrap();
    // A separately opened handle must still be refused while the guard lives.
    assert!(AppGate::acquire(&fixture.app, &fixture.home).is_err());
    drop(gate);
    // A duplicated open file description must not keep the lock after guard drop.
    let next = AppGate::acquire(&fixture.app, &fixture.home)
        .expect("AppGate drop did not release the installer lock with a cloned fd open");
    assert_eq!(next.recover().unwrap(), Recovery::Clean);
    drop(next);
    drop(inherited_fd);
}
