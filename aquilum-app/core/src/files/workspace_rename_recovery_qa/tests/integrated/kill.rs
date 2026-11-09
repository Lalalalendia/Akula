//! Controlled process-termination windows on a temporary vault.
//! This does not validate fsync/power-loss durability or authorize a rename.
//! Parent never invokes recovery writes. Child only touches a temp fixture.
use super::*;
use std::fs::OpenOptions;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const CHILD_FLAG: &str = "AKULA_QA_RENAME_KILL_CHILD";
const CHILD_EXACT: &str =
    "files::workspace_rename_recovery_qa::tests::integrated::kill::interrupted_rename_child";

struct KillOnDrop(Child);
impl KillOnDrop {
    fn kill_and_reap(&mut self) {
        if self.0.try_wait().unwrap().is_none() {
            self.0.kill().unwrap();
        }
        self.0.wait().unwrap();
    }
}
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn report(marker: &Path) {
    let mut f = OpenOptions::new()
        .write(true).create_new(true).open(marker).unwrap();
    f.write_all(b"checkpoint").unwrap();
    f.sync_all().unwrap();
}

#[test]
fn interrupted_rename_child() {
    if std::env::var_os(CHILD_FLAG).is_none() { return; }
    let source = PathBuf::from(std::env::var_os("AKULA_QA_RENAME_SOURCE").unwrap());
    let destination = PathBuf::from(std::env::var_os("AKULA_QA_RENAME_DEST").unwrap());
    let marker = PathBuf::from(std::env::var_os("AKULA_QA_RENAME_MARKER").unwrap());
    let phase = std::env::var("AKULA_QA_RENAME_PHASE").unwrap();
    if phase != "before" {
        no_replace_directory_rename(&source, &destination).unwrap();
        if phase == "after-cas" {
            // A separate process really commits the new path to the same
            // fixture SQLite before the hard kill; this is not a fake CAS.
            let database = PathBuf::from(std::env::var_os("AKULA_QA_RENAME_DB").unwrap());
            let id = std::env::var("AKULA_QA_RENAME_UUID").unwrap()
                .parse::<Uuid>().unwrap();
            let service = UiStateService::open(&database);
            service.replace_workspace_path_after_move(
                id, &source.to_string_lossy(), &destination.to_string_lossy(),
            ).unwrap();
        } else {
            assert_eq!(phase, "after");
        }
    }
    report(&marker);
    loop { thread::park_timeout(Duration::from_secs(60)); }
}

fn terminate_at(phase: &str, source: &Path, dest: &Path, marker: &Path,
                database: &Path, id: Uuid) {
    let child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact").arg(CHILD_EXACT).arg("--nocapture")
        .env(CHILD_FLAG, "1")
        .env("AKULA_QA_RENAME_SOURCE", source)
        .env("AKULA_QA_RENAME_DEST", dest)
        .env("AKULA_QA_RENAME_MARKER", marker)
        .env("AKULA_QA_RENAME_PHASE", phase)
        .env("AKULA_QA_RENAME_DB", database)
        .env("AKULA_QA_RENAME_UUID", id.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn().unwrap();
    let mut guard = KillOnDrop(child);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if fs::read(marker).is_ok_and(|b| b == b"checkpoint") {
            break;
        }
        assert!(guard.0.try_wait().unwrap().is_none(),
            "child terminated before the requested rename checkpoint");
        assert!(Instant::now() < deadline, "rename checkpoint timed out");
        thread::sleep(Duration::from_millis(20));
    }
    guard.kill_and_reap();
    assert!(marker.is_file());
}

fn check_hard_kill(phase: &str, expected: RecoveryView) {
    let (tmp, service, intent, journal) = with_registry();
    // Keep both note bytes and version history while the child is killed.
    let history = intent.source.join(".aquilum/history");
    fs::create_dir_all(&history).unwrap();
    fs::write(history.join("version.bin"), [0_u8, 200, 255]).unwrap();
    service.plan_inactive_workspace_rename(intent.workspace_id, "After", None).unwrap();
    write_new_journal(&journal, &intent).unwrap();
    let journal_bytes = fs::read(&journal).unwrap();
    let before = stored_path(&service, intent.workspace_id);
    let marker = tmp.path().join(format!("{phase}-checkpoint"));
    terminate_at(phase, &intent.source, &intent.destination, &marker,
        &tmp.path().join("ui-state.sqlite"), intent.workspace_id);
    // The crash must preserve the exact registry state reached by the child.
    let registered = if phase == "after-cas" { &intent.destination } else { &before };
    assert_eq!(stored_path(&service, intent.workspace_id), *registered);
    assert_eq!(classify_journal_with_registry(&journal, &full_registry(&service)), expected);
    assert_eq!(fs::read(&journal).unwrap(), journal_bytes);
    let current = if phase == "before" { &intent.source } else { &intent.destination };
    let absent = if phase == "before" { &intent.destination } else { &intent.source };
    assert!(!absent.exists());
    assert_eq!(fs::read_to_string(current.join("note.md")).unwrap(), "original");
    assert_eq!(fs::read(current.join(".aquilum/history/version.bin")).unwrap(), [0, 200, 255]);
}

#[test]
fn killed_before_native_rename_remains_prepared_without_change() {
    check_hard_kill("before", RecoveryView::Prepared);
}

#[test]
fn killed_after_native_rename_before_sqlite_cas_requires_review() {
    check_hard_kill("after", RecoveryView::MovedWaitingForRegistry);
}

#[test]
fn killed_after_sqlite_cas_before_journal_ack_requires_review() {
    check_hard_kill("after-cas", RecoveryView::RegistryChangedWaitingForAck);
}
