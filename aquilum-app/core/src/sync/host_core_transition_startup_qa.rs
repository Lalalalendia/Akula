//! Non-shipping regression: the transition-selected binary MUST NOT expose
//! the old, unbound Core::open writer from a second appdata/process.
//! All paths are isolated disposable test fixtures. This is a FAIL-CLOSED
//! default, not installed-host authorization or a production rename.
use crate::app_core::{Core, CoreEvent, EventSink};
use crate::files::gate;
use crate::history::{NoteWrite, Source};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

const CHILD: &str = "sync::host_core_transition_startup_qa::unbound_child";
const ENV: &str = "AKULA_QA_UNBOUND_CORE_CHILD";

fn events() -> Arc<dyn EventSink> {
    Arc::new(|_: CoreEvent| {})
}

fn unbound_core(appdata: &Path, root: &Path) -> Arc<Core> {
    fs::create_dir_all(appdata).unwrap();
    let core = Core::open(appdata, events());
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    assert!(core.requires_installed_host_epoch(),
        "transition-selected Core must require the host epoch even if no host DB was found");
    core
}

fn must_hold_every_ordinary_writer(core: &Core, root: &Path) {
    let note = root.join("Original.md");
    assert!(core.file_mutation_guard_for_write().is_err());
    assert!(core.file_mutation_guard_for_custody_write().is_err());
    assert!(gate::write(core, &note, "UNAUTHORIZED", None, Source::Me, None).is_err());
    assert!(gate::create(core, &root.join("Forbidden.md"),
        "UNAUTHORIZED", Source::Me, None).is_err());
    assert!(gate::create_binary(core, &root.join("Forbidden.bin"),
        b"UNAUTHORIZED").is_err());
    assert!(gate::copy(core, &note, &root.join("Copy.md")).is_err());
    assert!(core.history.try_saved(
        &|| core.known_roots(), &note,
        NoteWrite {
            disk_before: None,
            text: "UNAUTHORIZED", hash: "must-not-publish",
            source: Source::Me, name: None,
        },
    ).is_err());
    assert_eq!(fs::read(&note).unwrap(), b"original bytes");
    assert!(!root.join("Forbidden.md").exists());
    assert!(!root.join("Forbidden.bin").exists());
    assert!(!root.join("Copy.md").exists());
}

#[test]
fn default_core_open_never_admits_legacy_writes_with_transition_lab_selected() {
    let tmp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(tmp.path()).unwrap();
    let root = base.join("vault");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("Original.md"), b"original bytes").unwrap();
    let core = unbound_core(&base.join("unbound-appdata-a"), &root);
    must_hold_every_ordinary_writer(&core, &root);
}

#[test]
fn unbound_child() {
    let Some(flag) = std::env::var_os(ENV) else { return };
    assert_eq!(flag, "exercise");
    let root = PathBuf::from(std::env::var_os("AKULA_QA_UNBOUND_ROOT").unwrap());
    let appdata = PathBuf::from(std::env::var_os("AKULA_QA_UNBOUND_APPDATA").unwrap());
    let report = PathBuf::from(std::env::var_os("AKULA_QA_UNBOUND_REPORT").unwrap());
    let core = unbound_core(&appdata, &root);
    must_hold_every_ordinary_writer(&core, &root);
    fs::write(report, b"all-writers-held").unwrap();
}

#[test]
fn independent_appdata_and_runtime_host_env_cannot_reenable_unbound_writer() {
    let tmp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(tmp.path()).unwrap();
    let root = base.join("vault");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("Original.md"), b"original bytes").unwrap();
    let fake_host = base.join("runtime-host");
    fs::create_dir(&fake_host).unwrap();
    let misleading_db = fake_host.join("host.sqlite");
    // Installer fixture validates canonical forbidden roots. Create the
    // alternate appdata directory before provisioning the fake host ledger.
    let app_b = base.join("app-b");
    fs::create_dir(&app_b).unwrap();
    drop(crate::sync::host_custody_registry::SharedHostCustodyRegistry::provision_new(
        &misleading_db, &[root.as_path(), app_b.as_path()],
    ).unwrap());
    let report = base.join("child-report");
    let child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact").arg(CHILD).arg("--nocapture")
        .env(ENV, "exercise")
        .env("AKULA_QA_UNBOUND_ROOT", &root)
        .env("AKULA_QA_UNBOUND_APPDATA", base.join("app-b"))
        .env("AKULA_QA_UNBOUND_REPORT", &report)
        // Runtime environment cannot override the compile-time host choice.
        .env("AKULA_INSTALLED_HOST_CUSTODY_DB", &misleading_db)
        .stdout(Stdio::piped()).stderr(Stdio::piped())
        .output().unwrap();
    assert!(child.status.success(), "unbound child failed: stdout={}, stderr={}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr));
    assert_eq!(fs::read(&report).unwrap(), b"all-writers-held");
    assert_eq!(fs::read(root.join("Original.md")).unwrap(), b"original bytes");
    assert!(!root.join("Forbidden.md").exists());
    assert!(!root.join("Copy.md").exists());
}
