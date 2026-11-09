use super::*;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const CHILD: &str = "sync::workspace_transition_lease_qa::tests::child_entrypoint";
const CHILD_ENV: &str = "AKULA_TRANSITION_QA_CHILD";

fn setup() -> (tempfile::TempDir, PathBuf, PathBuf, Reservation) {
    let temp = tempfile::tempdir().unwrap();
    let parent = fs::canonicalize(temp.path()).unwrap();
    let source = parent.join("original-base");
    let destination = parent.join("renamed-base");
    let host = parent.join("host");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&host).unwrap();
    fs::write(source.join("chapter.md"), b"never alter bytes").unwrap();
    let ledger = host.join("transition-qa.sqlite");
    let _ = FixtureLedger::provision_fixture(&ledger).unwrap();
    let request = observed_request(Uuid::new_v4(), Uuid::new_v4(),
        &source, &destination).unwrap();
    (temp, ledger, source, request)
}

fn marker(path: &Path, contents: &[u8]) {
    let mut f = OpenOptions::new().create_new(true).write(true).open(path).unwrap();
    f.write_all(contents).unwrap();
    f.sync_all().unwrap();
}
fn wait_for(child: &mut Child, path: &Path, expected: &[u8]) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if fs::read(path).is_ok_and(|bytes| bytes.as_slice() == expected) { return; }
        assert!(child.try_wait().unwrap().is_none(), "child died before checkpoint");
        assert!(Instant::now() < deadline, "child checkpoint timed out");
        thread::sleep(Duration::from_millis(20));
    }
}
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn spawn(mode: &str, ledger: &Path, request: &Reservation, report: &Path) -> ChildGuard {
    let child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact").arg(CHILD).arg("--nocapture")
        .env(CHILD_ENV, mode)
        .env("AKULA_TRANSITION_QA_DB", ledger)
        .env("AKULA_TRANSITION_QA_SOURCE", &request.source)
        .env("AKULA_TRANSITION_QA_DEST", &request.destination)
        .env("AKULA_TRANSITION_QA_REPORT", report)
        .stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().unwrap();
    ChildGuard(child)
}

#[test]
fn child_entrypoint() {
    let Some(mode) = std::env::var_os(CHILD_ENV) else { return };
    let db = PathBuf::from(std::env::var_os("AKULA_TRANSITION_QA_DB").unwrap());
    let source = PathBuf::from(std::env::var_os("AKULA_TRANSITION_QA_SOURCE").unwrap());
    let dest = PathBuf::from(std::env::var_os("AKULA_TRANSITION_QA_DEST").unwrap());
    let report = PathBuf::from(std::env::var_os("AKULA_TRANSITION_QA_REPORT").unwrap());
    let mut ledger = FixtureLedger::open_existing(&db).unwrap();
    if mode == "hold-after-commit" {
        let request = observed_request(Uuid::new_v4(), Uuid::new_v4(), &source, &dest).unwrap();
        ledger.reserve(&request).unwrap();
        marker(&report, b"committed");
        loop { thread::park_timeout(Duration::from_secs(60)); }
    }
    if mode == "contend" {
        marker(&report.with_extension("ready"), b"ready");
        let request = observed_request(Uuid::new_v4(), Uuid::new_v4(), &source, &dest).unwrap();
        let result = ledger.reserve(&request);
        marker(&report, if result.is_err() { b"denied" } else { b"unsafe" });
        assert!(result.is_err(), "competing operation unexpectedly acquired names");
        return;
    }
    panic!("unknown QA child mode");
}

#[test]
fn pending_reservation_blocks_old_new_and_nested_paths_without_touching_files() {
    let (_tmp, db, source, request) = setup();
    let mut ledger = FixtureLedger::open_existing(&db).unwrap();
    ledger.reserve(&request).unwrap();
    assert!(ledger.would_hold(&request.source).unwrap());
    assert!(ledger.would_hold(&request.destination).unwrap());
    assert!(ledger.would_hold(&request.destination.join("nested")).unwrap());
    assert!(ledger.reserve(&request).is_err(), "replay cannot grant a second transition");
    let other = observed_request(Uuid::new_v4(), Uuid::new_v4(),
        &source, &source.with_file_name("other-destination")).unwrap();
    assert!(matches!(ledger.reserve(&other), Err(Denial::PathReserved)));
    assert_eq!(fs::read(source.join("chapter.md")).unwrap(), b"never alter bytes");
    assert!(!request.destination.exists());
}

#[test]
fn cold_restart_preserves_bilateral_reservation_after_child_is_killed() {
    let (temp, db, source, request) = setup();
    let report = temp.path().join("committed-checkpoint");
    let mut child = spawn("hold-after-commit", &db, &request, &report);
    wait_for(&mut child.0, &report, b"committed");
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    drop(child);
    let mut reopened = FixtureLedger::open_existing(&db).unwrap();
    let rows = reopened.inspect().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].source, request.source);
    assert_eq!(rows[0].destination, request.destination);
    assert!(reopened.would_hold(&request.source).unwrap());
    assert!(reopened.would_hold(&request.destination).unwrap());
    assert!(matches!(reopened.reserve(&request), Err(Denial::PathReserved)));
    assert_eq!(fs::read(source.join("chapter.md")).unwrap(), b"never alter bytes");
}

#[test]
fn separate_process_cannot_reserve_while_host_writer_epoch_is_held() {
    let (tmp, db, _source, request) = setup();
    let mut owner = FixtureLedger::open_existing(&db).unwrap();
    let tx = owner.connection.transaction_with_behavior(TransactionBehavior::Immediate).unwrap();
    let report = tmp.path().join("contended-checkpoint");
    let mut child = spawn("contend", &db, &request, &report);
    wait_for(&mut child.0, &report.with_extension("ready"), b"ready");
    // The child has opened the same DB and is trying to acquire BEGIN IMMEDIATE.
    // Keep our transaction alive until its busy timeout has elapsed.
    wait_for(&mut child.0, &report, b"denied");
    tx.commit().unwrap();
    assert!(child.0.wait().unwrap().success());
    assert!(owner.inspect().unwrap().is_empty());
    assert_eq!(fs::read(request.source.join("chapter.md")).unwrap(), b"never alter bytes");
}

#[test]
fn cold_reopen_second_core_cannot_take_reserved_destination() {
    let (tmp, db, source, request) = setup();
    let mut owner = FixtureLedger::open_existing(&db).unwrap();
    owner.reserve(&request).unwrap();
    let report = tmp.path().join("rejected-checkpoint");
    let mut child = spawn("contend", &db, &request, &report);
    wait_for(&mut child.0, &report, b"denied");
    assert!(child.0.wait().unwrap().success());
    assert_eq!(owner.inspect().unwrap().len(), 1);
    assert_eq!(fs::read(source.join("chapter.md")).unwrap(), b"never alter bytes");
}

#[test]
fn missing_or_damaged_host_fixture_never_becomes_an_empty_roster() {
    let (tmp, db, _source, request) = setup();
    let missing = tmp.path().join("missing-db");
    assert!(FixtureLedger::open_existing(&missing).is_err());
    assert!(!missing.exists());
    let mut owner = FixtureLedger::open_existing(&db).unwrap();
    owner.reserve(&request).unwrap();
    drop(owner);
    // Preserve the original journal; version mismatch must not reinitialize.
    let raw = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
    raw.execute_batch("PRAGMA user_version=99;").unwrap();
    drop(raw);
    assert!(FixtureLedger::open_existing(&db).is_err());
    assert!(db.exists());
}
