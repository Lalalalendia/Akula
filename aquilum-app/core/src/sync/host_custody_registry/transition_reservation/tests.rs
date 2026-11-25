use super::*;
use crate::app_core::{Core, CoreEvent, EventSink};
use crate::files::gate;
use crate::history::Source;
use crate::sync::VaultId;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const CHILD: &str =
    "sync::host_custody_registry::transition_reservation::tests::transition_child";
const ENV: &str = "AKULA_HOST_TRANSITION_CHILD";

fn events() -> Arc<dyn EventSink> {
    Arc::new(|_: CoreEvent| {})
}

struct Fixture {
    temp: tempfile::TempDir,
    db: PathBuf,
    root: PathBuf,
    dest: PathBuf,
    data: PathBuf,
    request: HostTransitionIntent,
}
fn setup() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let root = base.join("original-vault");
    let dest = base.join("renamed-vault");
    let host = base.join("shared-host");
    let data = base.join("core-a");
    for p in [&root, &host, &data] { fs::create_dir(p).unwrap(); }
    fs::write(root.join("chapter.md"), b"original note").unwrap();
    let db = host.join("installed.sqlite");
    let ledger = SharedHostCustodyRegistry::provision_new(
        &db, &[root.as_path(), data.as_path()],
    ).unwrap();
    drop(ledger);
    let (device, file) = PinnedProbeRoot::open(&root).unwrap().identity();
    let request = HostTransitionIntent {
        operation_id: Uuid::new_v4(),
        workspace_id: Uuid::new_v4(),
        source: root.clone(),
        destination: dest.clone(),
        source_identity: PhysicalObjectId { device, file },
    };
    Fixture { temp, db, root, dest, data, request }
}

fn registry(db: &Path) -> SharedHostCustodyRegistry {
    SharedHostCustodyRegistry::open_existing(db).unwrap()
}

fn core_at(data: &Path, db: &Path, root: &Path) -> Arc<Core> {
    fs::create_dir_all(data).unwrap();
    let core = Core::open_with_installed_host(data, db, events());
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    core
}
fn record_at(path: &Path, id: PhysicalObjectId, workspace_id: VaultId) -> super::super::HostCustodyRecord {
    super::super::HostCustodyRecord {
        root_path: path.to_string_lossy().into_owned(),
        root_identity: id,
        vault_id: workspace_id,
        profile_id: "approved-s3".into(),
        custody_scope_path: path.parent().unwrap().join("external-custody")
            .to_string_lossy().into_owned(),
        lock_identity: PhysicalObjectId { device: 1001, file: 2209 },
    }
}

#[test]
fn reservation_fences_installed_host_primary_and_history_without_mutating_notes() {
    let f = setup();
    let core = core_at(&f.data, &f.db, &f.root);
    let note = f.root.join("chapter.md");
    gate::write(&core, &note, "updated before transition", None, Source::Me, None).unwrap();
    let previous = fs::read(&note).unwrap();
    let history = core.history.page(&|| vec![f.root.clone()], &note, 0, 8);
    assert!(history.total > 0);
    let mut host = registry(&f.db);
    host.reserve_transition(&f.request).unwrap();
    assert_eq!(host.transition_snapshot().unwrap(), vec![f.request.clone()]);
    assert!(gate::write(&core, &note, "FORBIDDEN", None, Source::Me, None).is_err());
    assert!(gate::create(&core, &f.root.join("Forbidden.md"),
        "FORBIDDEN", Source::Me, None).is_err());
    assert!(core.history.try_saved(
        &|| core.known_roots(), &note,
        crate::history::NoteWrite {
            disk_before: None, text: "FORBIDDEN", hash: "bad",
            source: Source::Me, name: None,
        },
    ).is_err());
    assert_eq!(fs::read(&note).unwrap(), previous);
    assert!(!f.root.join("Forbidden.md").exists());
    assert_eq!(core.history.page(&|| vec![f.root.clone()], &note, 0, 8).total,
        history.total);
}

#[test]
fn reservation_refuses_enrollment_inspection_and_unenrolled_epoch_for_both_paths() {
    let f = setup();
    let mut host = registry(&f.db);
    host.reserve_transition(&f.request).unwrap();
    let requested = f.request.source_identity;
    assert!(matches!(host.inspect(&f.root, requested),
        Err(HostCustodyError::ConflictingEnrollment)));
    let mut called = false;
    assert!(matches!(
        host.with_unenrolled_epoch(&f.root, requested, || { called = true; }),
        Err(HostCustodyError::ConflictingEnrollment)
    ));
    assert!(!called);
    let record = record_at(&f.root, requested, VaultId::new());
    assert!(matches!(host.enroll(&record),
        Err(HostCustodyError::ConflictingEnrollment)));
    // Simulate a move on this TEMP fixture only. The physical root ID remains
    // the same; the new path must not escape host-wide reservation.
    fs::rename(&f.root, &f.dest).unwrap();
    assert!(matches!(host.inspect(&f.dest, requested),
        Err(HostCustodyError::ConflictingEnrollment)));
    let mut after_called = false;
    assert!(matches!(
        host.with_unenrolled_epoch(&f.dest, requested, || { after_called = true; }),
        Err(HostCustodyError::ConflictingEnrollment)
    ));
    assert!(!after_called);
    assert!(matches!(
        host.enroll(&record_at(&f.dest, requested, VaultId::new())),
        Err(HostCustodyError::ConflictingEnrollment)
    ));
    assert_eq!(fs::read(f.dest.join("chapter.md")).unwrap(), b"original note");
}

#[test]
fn conflicting_overlap_and_same_uuid_replay_cannot_reserve_twice() {
    let f = setup();
    let mut host = registry(&f.db);
    host.reserve_transition(&f.request).unwrap();
    assert!(matches!(host.reserve_transition(&f.request),
        Err(HostCustodyError::ConflictingEnrollment)));
    let mut overlapping = f.request.clone();
    overlapping.operation_id = Uuid::new_v4();
    overlapping.workspace_id = Uuid::new_v4();
    overlapping.destination = f.root.with_file_name("another-vault");
    assert!(matches!(host.reserve_transition(&overlapping),
        Err(HostCustodyError::ConflictingEnrollment)));
    assert!(host.transition_snapshot().unwrap().len() == 1);
    assert!(!f.dest.exists());
}

#[test]
fn schema_one_missing_transition_table_and_corrupt_record_hold_without_repair() {
    let f = setup();
    let mut host = registry(&f.db);
    host.reserve_transition(&f.request).unwrap();
    drop(host);
    // An indexed checksum mismatch must reject the ENTIRE shared ledger.
    let mut raw = Connection::open(&f.db).unwrap();
    raw.execute("UPDATE workspace_transitions SET record_blake3 = 'bad'", [])
        .unwrap();
    drop(raw);
    assert!(SharedHostCustodyRegistry::open_existing(&f.db).is_err());
    let raw = Connection::open(&f.db).unwrap();
    raw.execute_batch(
        "UPDATE workspace_transitions SET record_blake3 = 'bad';
         PRAGMA user_version = 1;"
    ).unwrap();
    drop(raw);
    // This lab refuses v1 and NEVER upgrades/clears data on an ordinary open.
    assert!(SharedHostCustodyRegistry::open_existing(&f.db).is_err());
    let version: i64 = Connection::open(&f.db).unwrap()
        .query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap();
    assert_eq!(version, 1);
    assert_eq!(fs::read(f.root.join("chapter.md")).unwrap(), b"original note");
}

fn checkpoint(path: &Path, content: &[u8]) {
    fs::write(path, content).unwrap();
}
fn wait_for(child: &mut Child, path: &Path, expected: &[u8]) {
    let deadline = Instant::now() + Duration::from_secs(16);
    loop {
        if fs::read(path).is_ok_and(|bytes| bytes == expected) { return; }
        assert!(child.try_wait().unwrap().is_none(), "child died before checkpoint");
        assert!(Instant::now() < deadline, "child checkpoint timed out");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn transition_child() {
    let Some(mode) = std::env::var_os(ENV) else { return };
    let root = PathBuf::from(std::env::var_os("AKULA_HOST_TRANSITION_ROOT").unwrap());
    let db = PathBuf::from(std::env::var_os("AKULA_HOST_TRANSITION_DB").unwrap());
    let report = PathBuf::from(std::env::var_os("AKULA_HOST_TRANSITION_REPORT").unwrap());
    let mut host = registry(&db);
    let (device, file) = PinnedProbeRoot::open(&root).unwrap().identity();
    let id = PhysicalObjectId { device, file };
    if mode == "attempt-write" {
        let data = PathBuf::from(std::env::var_os("AKULA_HOST_TRANSITION_DATA").unwrap());
        let core = core_at(&data, &db, &root);
        let note = root.join("chapter.md");
        let denied = gate::write(&core, &note, "FOREIGN", None, Source::Me, None).is_err()
            && gate::create(&core, &root.join("foreign.md"), "FOREIGN", Source::Me, None).is_err();
        checkpoint(&report, if denied { b"denied" } else { b"unsafe" });
        assert!(denied);
        return;
    }
    let request = HostTransitionIntent {
        operation_id: Uuid::new_v4(), workspace_id: Uuid::new_v4(),
        source: root.clone(), destination: root.with_file_name("renamed-vault"),
        source_identity: id,
    };
    if mode == "reserve-and-hold" {
        host.reserve_transition(&request).unwrap();
        checkpoint(&report, b"reserved");
        loop { thread::park_timeout(Duration::from_secs(60)); }
    }
    if mode == "reserve-after-epoch" {
        checkpoint(&report.with_extension("ready"), b"ready");
        host.reserve_transition(&request).unwrap();
        checkpoint(&report, b"reserved");
        return;
    }
    panic!("unknown child mode");
}

fn spawn_child(mode: &str, f: &Fixture, report: &Path, data: Option<&Path>) -> Child {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.arg("--exact").arg(CHILD).arg("--nocapture")
        .env(ENV, mode)
        .env("AKULA_HOST_TRANSITION_ROOT", &f.root)
        .env("AKULA_HOST_TRANSITION_DB", &f.db)
        .env("AKULA_HOST_TRANSITION_REPORT", report)
        .stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(data) = data {
        cmd.env("AKULA_HOST_TRANSITION_DATA", data);
    }
    cmd.spawn().unwrap()
}

#[test]
fn independently_opened_core_cannot_write_while_another_process_has_reserved() {
    let f = setup();
    let mut host = registry(&f.db);
    host.reserve_transition(&f.request).unwrap();
    let report = f.temp.path().join("denied");
    let mut child = spawn_child("attempt-write", &f, &report,
        Some(&f.temp.path().join("core-b")));
    wait_for(&mut child, &report, b"denied");
    assert!(child.wait().unwrap().success());
    assert_eq!(fs::read(f.root.join("chapter.md")).unwrap(), b"original note");
    assert!(!f.root.join("foreign.md").exists());
}

#[test]
fn hard_kill_of_reserver_preserves_both_paths_as_hold_after_cold_reopen() {
    let f = setup();
    let report = f.temp.path().join("reserved");
    let mut child = spawn_child("reserve-and-hold", &f, &report, None);
    wait_for(&mut child, &report, b"reserved");
    child.kill().unwrap();
    child.wait().unwrap();
    let mut reopened = registry(&f.db);
    assert_eq!(reopened.transition_snapshot().unwrap().len(), 1);
    let (device, file) = PinnedProbeRoot::open(&f.root).unwrap().identity();
    let mut entered = false;
    assert!(matches!(reopened.with_unenrolled_epoch(
        &f.root, PhysicalObjectId { device, file }, || { entered = true; }),
        Err(HostCustodyError::ConflictingEnrollment)
    ));
    assert!(!entered);
    assert_eq!(fs::read(f.root.join("chapter.md")).unwrap(), b"original note");
    assert!(!f.dest.exists());
}

#[test]
fn reservation_waits_for_existing_full_host_epoch_before_committing() {
    let f = setup();
    let mut host = registry(&f.db);
    let report = f.temp.path().join("after-epoch");
    let identity = f.request.source_identity;
    let mut child: Option<Child> = None;
    host.with_unenrolled_epoch(&f.root, identity, || {
        let mut pending = spawn_child("reserve-after-epoch", &f, &report, None);
        wait_for(&mut pending, &report.with_extension("ready"), b"ready");
        thread::sleep(Duration::from_millis(160));
        assert!(!report.exists(), "reservation committed while writer epoch was held");
        child = Some(pending);
    }).unwrap();
    let mut child = child.unwrap();
    wait_for(&mut child, &report, b"reserved");
    assert!(child.wait().unwrap().success());
    assert_eq!(host.transition_snapshot().unwrap().len(), 1);
    let mut entered = false;
    assert!(host.with_unenrolled_epoch(&f.root, identity, || { entered = true; }).is_err());
    assert!(!entered);
}
