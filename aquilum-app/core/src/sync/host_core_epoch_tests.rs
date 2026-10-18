//! P0-B2a real File Gate proof for an EXPLICITLY host-bound Core.
//!
//! This is NOT yet the default Core::open path; that path remains an unsafe
//! alternate-appdata release blocker until native startup binds every Core.

use super::filegate_lock_probe::{PinnedProbeRoot, LOCK_NAME};
use super::host_custody_registry::{
    HostCustodyRecord, PhysicalObjectId, SharedHostCustodyRegistry,
};
use super::VaultId;
use crate::app_core::{Core, CoreEvent};
use crate::files::gate;
use crate::history::{NoteWrite, Source};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const CHILD: &str = "sync::host_core_epoch_tests::host_core_child_entrypoint";

fn events() -> Arc<dyn crate::app_core::EventSink> {
    Arc::new(|_: CoreEvent| {})
}

fn prep() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let app_a = temp.path().join("app-a");
    let app_b = temp.path().join("app-b");
    let host = temp.path().join("installed-host");
    let external = temp.path().join("external-custody");
    for path in [&root, &app_a, &app_b, &host, &external] {
        fs::create_dir_all(path).unwrap();
    }
    let root = fs::canonicalize(root).unwrap();
    (temp, root, app_a, app_b, host.join("custody.sqlite"), external)
}

fn provision(db: &Path, root: &Path, app: &Path) -> SharedHostCustodyRegistry {
    SharedHostCustodyRegistry::provision_new(db, &[root, app]).unwrap()
}

#[test]
fn host_core_child_entrypoint() {
    let Some(mode) = std::env::var_os("AKULA_HOST_CORE_TEST_MODE") else {
        return;
    };
    assert_eq!(mode, "deny-after-enroll");
    let root = PathBuf::from(std::env::var_os("AKULA_HOST_CORE_ROOT").unwrap());
    let app_b = PathBuf::from(std::env::var_os("AKULA_HOST_CORE_APPDATA_B").unwrap());
    let db = PathBuf::from(std::env::var_os("AKULA_HOST_CORE_DB").unwrap());
    let report = PathBuf::from(std::env::var_os("AKULA_HOST_CORE_REPORT").unwrap());
    let core_b = Core::open_with_installed_host(&app_b, &db, events());
    core_b.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();

    let note = root.join("Original.md");
    let created = root.join("Forbidden.md");
    let overwritten = gate::write(
        &core_b, &note, "FORBIDDEN", None, Source::Me, None,
    ).is_ok();
    let created_ok = gate::create(
        &core_b, &created, "FORBIDDEN", Source::Me, None,
    ).is_ok();
    let direct_history = core_b.history.try_saved(
        &|| core_b.known_roots(), &note,
        NoteWrite {
            disk_before: None, text: "FORBIDDEN",
            hash: "not-authorized", source: Source::Me, name: None,
        }
    ).is_ok();
    let raw_binary = gate::create_binary(&core_b, &root.join("Other.bin"), b"bad").is_ok();
    fs::write(report, format!(
        "{overwritten}|{created_ok}|{direct_history}|{raw_binary}"
    )).unwrap();
}

#[test]
fn host_bound_new_appdata_denies_enrolled_second_process_even_with_live_external_lock() {
    let (temp, root, app_a, app_b, db, external) = prep();
    let first = root.join("Original.md");
    fs::write(&first, b"unchanged").unwrap();
    let original = fs::read(&first).unwrap();
    let a = Core::open(&app_a, events());
    a.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();

    let vault_id = VaultId::new();
    a.documents.enroll_guarded_participant(
        &root, &vault_id.to_string(),
        &app_a.join("profiles.sqlite"), &app_a.join("custody.sqlite"),
        &external,
    ).unwrap();
    fs::write(external.join(LOCK_NAME), b"").unwrap();
    let external_root = PinnedProbeRoot::open(&external).unwrap();
    let held = external_root.try_lock().unwrap().expect("A has live OS custody lock");
    let physical = PinnedProbeRoot::open(&root).unwrap().identity();
    let mut host = provision(&db, &root, &app_a);
    host.enroll(&HostCustodyRecord {
        root_path: fs::canonicalize(&root).unwrap().to_string_lossy().into_owned(),
        root_identity: PhysicalObjectId { device: physical.0, file: physical.1 },
        vault_id,
        profile_id: "approved-s3".into(),
        custody_scope_path: fs::canonicalize(&external).unwrap().to_string_lossy().into_owned(),
        lock_identity: PhysicalObjectId { device: held.identity().0, file: held.identity().1 },
    }).unwrap();

    let report = temp.path().join("child-report");
    let result = Command::new(std::env::current_exe().unwrap())
        .arg("--exact").arg(CHILD).arg("--nocapture")
        .env("AKULA_HOST_CORE_TEST_MODE", "deny-after-enroll")
        .env("AKULA_HOST_CORE_ROOT", &root)
        .env("AKULA_HOST_CORE_APPDATA_B", &app_b)
        .env("AKULA_HOST_CORE_DB", &db)
        .env("AKULA_HOST_CORE_REPORT", &report)
        .output().unwrap();
    assert!(result.status.success(),
        "child OS process failed: stdout={}, stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr));
    assert_eq!(fs::read_to_string(report).unwrap(), "false|false|false|false");
    assert_eq!(fs::read(&first).unwrap(), original);
    assert!(!root.join("Forbidden.md").exists());
    assert!(!root.join("Other.bin").exists());
    assert!(a.documents.requires_guarded_participant_hold());
    drop(held);
}

#[test]
fn host_bound_core_allows_unenrolled_create_and_write_with_history_epoch() {
    let (_tmp, root, app_a, _app_b, db, _external) = prep();
    let _host = provision(&db, &root, &app_a);
    let core = Core::open_with_installed_host(&app_a, &db, events());
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    let note = root.join("Note.md");
    gate::create(&core, &note, "first", Source::Me, None).unwrap();
    gate::write(&core, &note, "second", None, Source::Me, None).unwrap();
    assert_eq!(fs::read_to_string(&note).unwrap(), "second");
    // An ordinary direct history call cannot use the transaction merely
    // because its Core also has the same appdata and host source.
    assert!(core.history.try_saved(
        &|| core.known_roots(), &note,
        NoteWrite {
            disk_before: None, text: "unauthorized",
            hash: "bad", source: Source::Me, name: None,
        }
    ).is_err());
    // Every File Gate route not yet covered by the host epoch is HOLD.
    assert!(gate::create_binary(&core, &root.join("Extra.bin"), b"bad").is_err());
    assert!(gate::copy(&core, &note, &root.join("Copy.md")).is_err());
    assert_eq!(fs::read_to_string(&note).unwrap(), "second");
    assert!(!root.join("Extra.bin").exists());
}

#[test]
fn missing_host_registry_is_not_an_empty_unenrolled_roster() {
    let (_tmp, root, app_a, _app_b, db, _external) = prep();
    let core = Core::open_with_installed_host(&app_a, &db, events());
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    let note = root.join("NoRegistry.md");
    assert!(gate::create(&core, &note, "forbidden", Source::Me, None).is_err());
    assert!(!note.exists());
    assert!(!db.exists(), "a Core must never provision a missing installer ledger");
}
