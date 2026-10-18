//! P0-B4b: a positive native, two-OS-process installed-host admission proof.
//!
//! A trusted test fixture (not this executable) provisions an empty shared
//! installer ledger at a build-time path. The native Workspace::open entrypoint
//! must still permit ordinary create/write + history for an UNENROLLED vault.
//! The exact same physical note is subsequently edited in a second OS
//! process under different appdata, using the same build-time host ledger.
//!
//! This is not OS-attested host ownership, S3 enrolled-root authorization,
//! installed release packaging, or interruption recovery.
use super::*;
use aquilum_core::files::error::FileCommandError;
use aquilum_core::files::gate;
use aquilum_core::history::Source;
use std::fs;
use std::process::Command;

const CHILD: &str = "workspace::installed_host_positive_tests::child_save";
const INITIAL: &str = "Первая версия\nUnicode 😀漢字\n";
const SECOND: &str = "Вторая версия через другой процесс\n😀漢字\n";

fn fixture_path() -> Option<PathBuf> {
    if std::env::var_os("AKULA_TEST_NATIVE_HOST_POSITIVE").is_none() {
        return None;
    }
    let built = option_env!("AKULA_INSTALLED_HOST_CUSTODY_DB")
        .expect("positive native CI must inject installer ledger during compilation");
    let path = PathBuf::from(built);
    assert!(path.is_absolute(), "installer ledger must have an absolute build-time path");
    assert_eq!(
        path.file_name().and_then(|x| x.to_str()),
        Some("akula-ci-native-positive-host.sqlite3"),
        "only the disposable named CI fixture may be used in this positive test",
    );
    assert!(
        path.is_file(),
        "installer fixture must exist BEFORE Core::open; never auto-provision it",
    );
    Some(path)
}

fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[test]
fn child_save() {
    if std::env::var_os("AKULA_NATIVE_POSITIVE_CHILD").is_none() {
        return;
    }
    let _ledger = fixture_path().expect("the native positive fixture must be enabled");
    let appdata = PathBuf::from(std::env::var_os("AKULA_NATIVE_POSITIVE_APPDATA").unwrap());
    let root = PathBuf::from(std::env::var_os("AKULA_NATIVE_POSITIVE_ROOT").unwrap());
    let old_digest = std::env::var("AKULA_NATIVE_POSITIVE_OLD_HASH").unwrap();
    let report = PathBuf::from(std::env::var_os("AKULA_NATIVE_POSITIVE_REPORT").unwrap());

    let workspace = Workspace::open(Some(&appdata), Some(&root), || {}).unwrap();
    assert_eq!(fs::canonicalize(workspace.root.as_deref().unwrap()).unwrap(), root);
    let note = root.join("Shared.md");
    assert_eq!(fs::read(&note).unwrap(), INITIAL.as_bytes());
    let new = gate::write(
        &workspace.core, &note, SECOND,
        Some(&old_digest), Source::Me, None,
    ).expect("native second OS process must admit enrolled-ledger UNENROLLED-vault write");
    assert_eq!(new.hash, digest(SECOND.as_bytes()));
    assert_eq!(fs::read(&note).unwrap(), SECOND.as_bytes());
    let page = workspace.core.history.page(&|| vec![root.clone()], &note, 0, 10);
    assert_eq!(page.total, 2, "both full history revisions must survive process boundary");

    fs::write(report, new.hash).unwrap();
}

#[test]
fn two_native_workspace_processes_share_installer_ledger_and_preserve_cas_history() {
    let Some(ledger) = fixture_path() else {
        return;
    };

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let app_a = temp.path().join("app-a");
    let app_b = temp.path().join("app-b");
    for dir in [&root, &app_a, &app_b] {
        fs::create_dir(dir).unwrap();
    }
    let root = fs::canonicalize(root).unwrap();
    assert!(!ledger.starts_with(&root) && !ledger.starts_with(&app_a) && !ledger.starts_with(&app_b));

    let a = Workspace::open(Some(&app_a), Some(&root), || {}).unwrap();
    assert_eq!(fs::canonicalize(a.root.as_deref().unwrap()).unwrap(), root);
    let note = root.join("Shared.md");
    let initial = gate::create(&a.core, &note, INITIAL, Source::Me, None)
        .expect("native installed-host must admit legitimate unenrolled note creation");
    assert_eq!(initial.hash, digest(INITIAL.as_bytes()));
    assert_eq!(fs::read(&note).unwrap(), INITIAL.as_bytes());
    assert_eq!(
        a.core.history.page(&|| vec![root.clone()], &note, 0, 10).total, 1,
        "initial version must be durably committed before another process opens",
    );

    let report = temp.path().join("child-result");
    let fake = temp.path().join("forged-runtime-host.sqlite");
    fs::write(&fake, b"not a trusted ledger").unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .arg("--exact").arg(CHILD).arg("--nocapture")
        .env("AKULA_TEST_NATIVE_HOST_POSITIVE", "1")
        .env("AKULA_NATIVE_POSITIVE_CHILD", "1")
        .env("AKULA_NATIVE_POSITIVE_APPDATA", &app_b)
        .env("AKULA_NATIVE_POSITIVE_ROOT", &root)
        .env("AKULA_NATIVE_POSITIVE_OLD_HASH", &initial.hash)
        .env("AKULA_NATIVE_POSITIVE_REPORT", &report)
        .env("AKULA_INSTALLED_HOST_CUSTODY_DB", &fake)
        .output().unwrap();
    assert!(
        output.status.success(),
        "second native process failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("workspace::installed_host_positive_tests::child_save ... ok"),
        "child harness did not execute the intended native test: {}",
        String::from_utf8_lossy(&output.stdout),
    );
    let second_hash = digest(SECOND.as_bytes());
    assert_eq!(fs::read_to_string(&report).unwrap(), second_hash);
    assert_eq!(fs::read(&note).unwrap(), SECOND.as_bytes());
    assert_eq!(fs::read(&fake).unwrap(), b"not a trusted ledger");

    // The original process still holds its previously admitted Core, but may
    // NOT replace the second process's content from a stale disk revision.
    let error = gate::write(
        &a.core, &note, "stale overwrite must not land",
        Some(&initial.hash), Source::Me, None,
    ).expect_err("stale first-process CAS must refuse");
    assert!(matches!(error, FileCommandError::Conflict { .. }), "{error:?}");
    assert_eq!(fs::read(&note).unwrap(), SECOND.as_bytes());

    // Cold appdata A readback: no in-process HistoryService cache should be
    // needed to recover either durable version after another process edited.
    drop(a);
    let reopened = Workspace::open(Some(&app_a), Some(&root), || {}).unwrap();
    let page = reopened.core.history.page(&|| vec![root.clone()], &note, 0, 10);
    assert_eq!(page.total, 2, "a refused stale CAS must not append history");
    let versions = page.versions.iter().map(|v| {
        reopened.core.history.version(&|| vec![root.clone()], &note, Some(&v.id))
            .expect("history must remain readable after native reopen").text
    }).collect::<Vec<_>>();
    assert!(versions.iter().any(|v| v == INITIAL));
    assert!(versions.iter().any(|v| v == SECOND));
    assert_eq!(digest(&fs::read(&note).unwrap()), second_hash);
}
