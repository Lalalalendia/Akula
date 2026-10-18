//! P0-B6b-1: actual native Workspace::open, not merely direct Core fixture.
//! Consumes the disposable independent Core witness created by the previous
//! stage of the SAME CI job. Neither stage authorizes production S3 writes.

use super::*;
use aquilum_core::files::gate;
use aquilum_core::history::Source;
use std::fs;
use std::process::Command;

const CHILD: &str = "workspace::installed_host_witness_tests::native_witness_child";

#[test]
fn native_witness_child() {
    if std::env::var_os("AKULA_B6B_NATIVE_CHILD").is_none() {
        return;
    }
    let appdata = PathBuf::from(std::env::var_os("AKULA_B6B_NATIVE_APPDATA").unwrap());
    let root = PathBuf::from(std::env::var_os("AKULA_B6B_NATIVE_ROOT").unwrap());
    let report = PathBuf::from(std::env::var_os("AKULA_B6B_NATIVE_REPORT").unwrap());
    let workspace = Workspace::open(Some(&appdata), Some(&root), || {}).unwrap();
    let note = root.join("ActualNative.md");
    let result = gate::write(
        &workspace.core, &note, "native child update",
        None, Source::Me, None,
    );
    fs::write(&report, if result.is_ok() { "allowed" } else { "hold" }).unwrap();
}

#[test]
fn real_native_workspace_two_processes_reject_removed_compiled_witness() {
    if std::env::var_os("AKULA_B6B_NATIVE_SMOKE").is_none() {
        return;
    }
    let db = PathBuf::from(option_env!("AKULA_INSTALLED_HOST_CUSTODY_DB")
        .expect("dedicated witnessed-host CI needs build-time DB"));
    let witness = PathBuf::from(option_env!("AKULA_INSTALLED_HOST_WITNESS_PATH")
        .expect("dedicated witnessed-host CI needs build-time receipt"));
    assert!(db.is_file() && witness.is_file(),
        "native smoke must reuse independently provisioned prior Core fixture");
    assert_eq!(db.file_name().and_then(|s| s.to_str()),
        Some("akula-ci-b6b-ledger.sqlite3"));
    assert_eq!(witness.file_name().and_then(|s| s.to_str()),
        Some("akula-ci-b6b-receipt.json"));

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("native-vault");
    let a = temp.path().join("native-app-A");
    let b = temp.path().join("native-app-B");
    for path in [&root, &a, &b] { fs::create_dir(path).unwrap(); }
    let root = fs::canonicalize(root).unwrap();
    let first = Workspace::open(Some(&a), Some(&root), || {}).unwrap();
    let note = root.join("ActualNative.md");
    gate::create(&first.core, &note, "native initial",
        Source::Me, None).expect("valid witness must admit native note");
    assert_eq!(fs::read_to_string(&note).unwrap(), "native initial");
    drop(first);

    let report = temp.path().join("native-child-result.txt");
    let fake = temp.path().join("runtime-receipt-is-not-authority");
    fs::write(&fake, b"forged").unwrap();
    let child = || {
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact").arg(CHILD).arg("--nocapture")
            .env("AKULA_B6B_NATIVE_CHILD", "1")
            .env("AKULA_B6B_NATIVE_APPDATA", &b)
            .env("AKULA_B6B_NATIVE_ROOT", &root)
            .env("AKULA_B6B_NATIVE_REPORT", &report)
            .env("AKULA_INSTALLED_HOST_WITNESS_PATH", &fake)
            .env("AKULA_INSTALLED_HOST_CUSTODY_DB", &fake)
            .output().unwrap();
        assert!(output.status.success(),
            "native child failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stdout)
            .contains("native_witness_child ... ok"),
            "native child did not run expected test: {}",
            String::from_utf8_lossy(&output.stdout));
        fs::read_to_string(&report).unwrap()
    };
    assert_eq!(child(), "allowed");
    assert_eq!(fs::read_to_string(&note).unwrap(), "native child update");

    let parked = witness.with_file_name("akula-ci-b6b-receipt-parked.json");
    fs::rename(&witness, &parked).unwrap();
    assert_eq!(child(), "hold", "native must not write without receipt");
    assert_eq!(fs::read_to_string(&note).unwrap(), "native child update");
    fs::rename(&parked, &witness).unwrap();
    assert_eq!(child(), "allowed");
    assert_eq!(fs::read_to_string(&note).unwrap(), "native child update");

    let reopened = Workspace::open(Some(&a), Some(&root), || {}).unwrap();
    assert!(reopened.core.history.page(&|| vec![root.clone()],
        &note, 0, 10).total >= 2);
}
