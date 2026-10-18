//! Installed-host binary contract: no explicit installer registry must be
//! observationally read-only, even with a forged runtime environment value.
//! These tests execute ONLY when the opt-in sync-installed-host feature is on.
//! A positive, enrolled host DB with explicit Core lives in host_core_epoch_tests.

use super::{Core, CoreEvent};
use crate::files::gate;
use crate::history::{NoteWrite, Source};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const CHILD: &str = "app_core::installed_host_tests::mandatory_host_default_core_child";

fn events() -> Arc<dyn super::EventSink> {
    Arc::new(|_: CoreEvent| {})
}

fn lacks_compiled_installer_path() -> bool {
    !option_env!("AKULA_INSTALLED_HOST_CUSTODY_DB")
        .map(Path::new)
        .is_some_and(|p| p.is_absolute() && p.file_name().is_some())
}

#[test]
fn mandatory_host_default_core_child() {
    if std::env::var_os("AKULA_INSTALLED_HOST_CHILD").is_none() {
        return;
    }
    let app = PathBuf::from(std::env::var_os("AKULA_INSTALLED_HOST_APPDATA").unwrap());
    let root = PathBuf::from(std::env::var_os("AKULA_INSTALLED_HOST_ROOT").unwrap());
    let report = PathBuf::from(std::env::var_os("AKULA_INSTALLED_HOST_REPORT").unwrap());
    let core = Core::open(&app, events());
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    let orig = root.join("Original.md");
    let bad = root.join("ChildUnauthorized.md");
    let write = gate::write(&core, &orig, "forbidden", None, Source::Me, None).is_ok();
    let create = gate::create(&core, &bad, "forbidden", Source::Me, None).is_ok();
    let binary = gate::create_binary(&core, &root.join("Other.bin"), b"forbidden").is_ok();
    fs::write(report, format!("{write}|{create}|{binary}")).unwrap();
}

#[test]
fn mandatory_installed_host_core_open_has_no_appdata_fallback() {
    // In a correctly provisioned installer build, positive behavior depends
    // on the pinned build-time DB path and its OS policy. This test proves
    // fail-closed behavior for *missing* installer authority, and CI compiles
    // this feature with AKULA_INSTALLED_HOST_CUSTODY_DB="".
    if !lacks_compiled_installer_path() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let a = temp.path().join("app-a");
    let b = temp.path().join("app-b");
    for path in [&root, &a, &b] { fs::create_dir(path).unwrap(); }
    let root = fs::canonicalize(root).unwrap();
    let original = root.join("Original.md");
    fs::write(&original, "unchanged").unwrap();

    let first = Core::open(&a, events());
    assert!(first.requires_installed_host_epoch());
    first.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    assert!(gate::write(
        &first, &original, "forbidden", None, Source::Me, None
    ).is_err());
    assert!(gate::create(
        &first, &root.join("Unauthorized.md"), "forbidden", Source::Me, None
    ).is_err());
    assert!(gate::copy(
        &first, &original, &root.join("Copy.md")
    ).is_err());
    assert!(first.empty_trash_guarded(&root).is_err());
    assert!(first.history.try_saved(
        &|| first.known_roots(),
        &original,
        NoteWrite {
            disk_before: None,
            text: "not authorized",
            hash: "unauthorized-hash",
            source: Source::Me,
            name: None,
        }
    ).is_err());
    assert_eq!(fs::read_to_string(&original).unwrap(), "unchanged");

    // Real B process: changing appdata and offering a runtime-env location
    // must NOT change the compile-time host policy.
    let report = temp.path().join("child-result");
    let forged = temp.path().join("forged-user-ledger.sqlite");
    fs::write(&forged, b"not an installer ledger").unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .arg("--exact").arg(CHILD).arg("--nocapture")
        .env("AKULA_INSTALLED_HOST_CHILD", "1")
        .env("AKULA_INSTALLED_HOST_APPDATA", &b)
        .env("AKULA_INSTALLED_HOST_ROOT", &root)
        .env("AKULA_INSTALLED_HOST_REPORT", &report)
        .env("AKULA_INSTALLED_HOST_CUSTODY_DB", &forged)
        .output().unwrap();
    assert!(output.status.success(),
        "independent OS process failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr));
    assert_eq!(fs::read_to_string(&report).unwrap(), "false|false|false",
        "installed-host build must not fall back to private appdata or runtime env");
    assert_eq!(fs::read_to_string(&original).unwrap(), "unchanged");
    assert!(!root.join("ChildUnauthorized.md").exists());
    assert!(!root.join("Other.bin").exists());
    assert!(!root.join("Unauthorized.md").exists());
    assert!(!root.join("Copy.md").exists());
}

#[test]
fn mandatory_installed_host_core_open_uses_real_compiled_shared_ledger() {
    use crate::sync::filegate_lock_probe::{PinnedProbeRoot, LOCK_NAME};
    use crate::sync::host_custody_registry::{
        HostCustodyRecord, PhysicalObjectId, SharedHostCustodyRegistry,
    };
    use crate::sync::VaultId;

    // The dedicated CI lane injects this exact, non-appdata path DURING
    // compilation. Don't create any file at a human/operator's build path.
    if std::env::var_os("AKULA_INSTALLED_HOST_TEST_ALLOW_PROVISION").is_none() {
        return;
    }
    let Some(build_path) = option_env!("AKULA_INSTALLED_HOST_CUSTODY_DB") else {
        panic!("CI provisioning test requires build-time installer path");
    };
    let host_db = PathBuf::from(build_path);
    assert!(host_db.is_absolute());
    assert_eq!(host_db.file_name().and_then(|x| x.to_str()),
        Some("akula-ci-installed-host.sqlite3"),
        "test may only provision the dedicated disposable CI host ledger");
    assert!(!host_db.exists(), "CI host ledger must start absent; never replace prior state");

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("enrolled-vault");
    let app_a = temp.path().join("app-a");
    let app_b = temp.path().join("app-b");
    let custody = temp.path().join("external-custody");
    for dir in [&root, &app_a, &app_b, &custody] {
        fs::create_dir(dir).unwrap();
    }
    let root = fs::canonicalize(root).unwrap();
    let original = root.join("Original.md");
    fs::write(&original, "original").unwrap();
    fs::write(custody.join(LOCK_NAME), b"").unwrap();
    let custody_root = PinnedProbeRoot::open(&custody).unwrap();
    let held = custody_root.try_lock().unwrap().expect("test external OS lock");
    let identity = PinnedProbeRoot::open(&root).unwrap().identity();

    let mut host = SharedHostCustodyRegistry::provision_new(
        &host_db, &[root.as_path(), app_a.as_path(), app_b.as_path()]
    ).unwrap();
    host.enroll(&HostCustodyRecord {
        root_path: fs::canonicalize(&root).unwrap().to_string_lossy().into_owned(),
        root_identity: PhysicalObjectId {
            device: identity.0, file: identity.1,
        },
        vault_id: VaultId::new(),
        profile_id: "approved-s3".into(),
        custody_scope_path: fs::canonicalize(&custody).unwrap()
            .to_string_lossy().into_owned(),
        lock_identity: PhysicalObjectId {
            device: held.identity().0, file: held.identity().1,
        },
    }).unwrap();
    drop(host);

    // A and B both call the PUBLIC, default Core::open. Only the compiled
    // installer feature makes both use the exact same host custody ledger.
    let a = Core::open(&app_a, events());
    assert!(a.requires_installed_host_epoch());
    a.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    assert!(gate::write(
        &a, &original, "owner-forbidden", None, Source::Me, None
    ).is_err());

    // A real installed host ledger must support legitimate writes in an
    // UNENROLLED sibling workspace, not simply deny every operation.
    // This is the positive counterpart to the protected-root refusal.
    let clean_root = temp.path().join("unenrolled-vault");
    fs::create_dir(&clean_root).unwrap();
    let clean_root = fs::canonicalize(clean_root).unwrap();
    a.ui_state.resolve_workspace(clean_root.to_str().unwrap(), 2).unwrap();
    let clean_note = clean_root.join("Allowed.md");
    gate::create(&a, &clean_note, "initial", Source::Me, None).unwrap();
    gate::write(&a, &clean_note, "updated", None, Source::Me, None).unwrap();
    assert_eq!(fs::read_to_string(&clean_note).unwrap(), "updated");
    assert!(a.history.page(&|| vec![clean_root.clone()], &clean_note, 0, 10).total > 0,
        "legitimate installed-host note publication must persist HistoryService");
    // An existing shared ledger permits CAS write/create in an UNENROLLED
    // workspace; it does NOT grant blanket filesystem or history authority.
    // Exercise the public File Gate routes in the same positive CI matrix.
    assert!(gate::create_binary(&a, &clean_root.join("Blocked.bin"), b"bad").is_err());
    assert!(gate::copy(&a, &clean_note, &clean_root.join("Copied.md")).is_err());
    assert!(gate::create_folder(&a, &clean_root.join("Folder")).is_err());
    assert!(gate::ensure_folder_tree(&a, &clean_root.join("Nested").join("Child")).is_err());
    assert!(gate::rename(&a, &clean_note, &clean_root.join("Renamed.md")).is_err());
    assert!(gate::move_across(&a, &clean_note, &clean_root.join("Moved.md")).is_err());
    assert!(gate::ensure_trash(&a, &clean_root).is_err());
    assert!(gate::trash(&a, &clean_root, &clean_note).is_err());
    assert!(a.empty_trash_guarded(&clean_root).is_err());
    assert!(gate::name_version(&a, &clean_note, None, "must-not-rename").is_none());
    assert_eq!(fs::read_to_string(&clean_note).unwrap(), "updated");
    for forbidden in ["Blocked.bin", "Copied.md", "Folder", "Nested",
                      "Renamed.md", "Moved.md", ".trash"] {
        assert!(!clean_root.join(forbidden).exists(),
            "installed-host unexpectedly changed a disallowed route: {forbidden}");
    }


    let report = temp.path().join("second-process-report");
    let output = Command::new(std::env::current_exe().unwrap())
        .arg("--exact").arg(CHILD).arg("--nocapture")
        .env("AKULA_INSTALLED_HOST_CHILD", "1")
        .env("AKULA_INSTALLED_HOST_APPDATA", &app_b)
        .env("AKULA_INSTALLED_HOST_ROOT", &root)
        .env("AKULA_INSTALLED_HOST_REPORT", &report)
        // Runtime attempt to point B to a different location is ignored.
        .env("AKULA_INSTALLED_HOST_CUSTODY_DB",
            temp.path().join("forged-b-ledger.sqlite"))
        .output().unwrap();
    assert!(output.status.success(),
        "child failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr));
    assert_eq!(fs::read_to_string(report).unwrap(), "false|false|false");
    assert_eq!(fs::read_to_string(&original).unwrap(), "original");
    assert!(!root.join("ChildUnauthorized.md").exists());
    assert!(!root.join("Other.bin").exists());
    drop(held);
}


#[test]
fn explicit_constructor_cannot_select_a_private_empty_host_database() {
    // CI compiles with an empty trusted installer location. A caller cannot
    // substitute its own *valid* empty host registry and regain File Gate
    // write authority inside an installed-host product build.
    if !lacks_compiled_installer_path() {
        return;
    }
    use crate::sync::host_custody_registry::SharedHostCustodyRegistry;

    let temp = tempfile::tempdir().unwrap();
    let vault = temp.path().join("vault");
    let appdata = temp.path().join("appdata-b");
    let fake_host_dir = temp.path().join("fake-host");
    for path in [&vault, &appdata, &fake_host_dir] {
        fs::create_dir_all(path).unwrap();
    }
    let vault = fs::canonicalize(vault).unwrap();
    let fake_db = fake_host_dir.join("private-empty.sqlite");
    let registry = SharedHostCustodyRegistry::provision_new(
        &fake_db, &[vault.as_path(), appdata.as_path()]
    ).unwrap();
    drop(registry);

    let b = Core::open_with_installed_host(&appdata, &fake_db, events());
    b.ui_state.resolve_workspace(vault.to_str().unwrap(), 1).unwrap();
    assert!(b.requires_installed_host_epoch());
    let note = vault.join("MustNotCreate.md");
    assert!(gate::create(&b, &note, "forbidden", Source::Me, None).is_err(),
        "installed build accepted a caller-chosen empty registry");
    assert!(!note.exists());
}


#[test]
fn missing_installer_ledger_blocks_all_structural_file_gate_mutations() {
    use crate::files::error::FileCommandError;
    use std::time::{Duration, SystemTime};

    // The other CI lane compiles a disposable real host DB and proves
    // ordinary writes can still succeed. This lane has NO compiler-issued
    // registry, so every structural/file-history mutation must remain HOLD.
    if !lacks_compiled_installer_path() {
        return;
    }

    fn expect_host_hold<T>(operation: &str, result: Result<T, FileCommandError>) {
        match result {
            Err(FileCommandError::Task { message })
                if message.contains("host custody is HOLD") => {}
            Err(error) => panic!("{operation} failed for a reason OTHER than host custody: {error:?}"),
            Ok(_) => panic!("{operation} escaped mandatory host custody without a ledger"),
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let appdata = temp.path().join("appdata");
    let root = temp.path().join("vault");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&appdata).unwrap();
    let root = fs::canonicalize(root).unwrap();
    let original = root.join("Keep.md");
    fs::write(&original, "unchanged primary").unwrap();

    let core = Core::open(&appdata, events());
    assert!(core.requires_installed_host_epoch());
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();

    // Public File Gate routes should refuse at custody admission, not only
    // due to missing source/destination or history preflight errors.
    expect_host_hold("create_binary",
        gate::create_binary(&core, &root.join("Image.bin"), b"forbidden"));
    expect_host_hold("copy",
        gate::copy(&core, &original, &root.join("Copy.md")));
    expect_host_hold("create_folder",
        gate::create_folder(&core, &root.join("NewFolder")));
    expect_host_hold("ensure_folder_tree",
        gate::ensure_folder_tree(&core, &root.join("Generated").join("Nested")));
    expect_host_hold("rename",
        gate::rename(&core, &original, &root.join("Renamed.md")));
    expect_host_hold("move_across",
        gate::move_across(&core, &original, &root.join("Moved.md")));
    expect_host_hold("ensure_trash",
        gate::ensure_trash(&core, &root));
    expect_host_hold("trash",
        gate::trash(&core, &root, &original));

    // Stage a real trash item using *test-only* direct fs operations. The
    // restore and retention calls below go through the public File Gate.
    let stale = root.join(".trash").join("Old").join("stale.md");
    fs::create_dir_all(stale.parent().unwrap()).unwrap();
    fs::write(&stale, "preserve this trash record").unwrap();
    fs::File::options().write(true).open(&stale).unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(40 * 24 * 3600))
        .unwrap();
    let deletions = crate::files::deletions::deletion_page_impl(&root, 0, 10).unwrap();
    assert!(deletions.total > 0, "fixture must contain a genuine deleted note");

    expect_host_hold("restore",
        gate::restore(&core, &root, &stale));
    expect_host_hold("restore_deletion",
        gate::restore_deletion(&core, &root, &deletions.deletions[0].id));
    expect_host_hold("cleanup_retention",
        gate::cleanup_retention(&core, &root, 30));
    expect_host_hold("empty_trash_guarded",
        core.empty_trash_guarded(&root));

    assert_eq!(fs::read_to_string(&original).unwrap(), "unchanged primary");
    assert_eq!(fs::read_to_string(&stale).unwrap(), "preserve this trash record");
    for forbidden in ["Image.bin", "Copy.md", "NewFolder", "Generated",
                      "Renamed.md", "Moved.md"] {
        assert!(!root.join(forbidden).exists(),
            "forbidden File Gate route changed bytes: {forbidden}");
    }
}
