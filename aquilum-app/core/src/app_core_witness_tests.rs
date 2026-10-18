// P0-B6b-1: actual compiled default Core::open and separate OS-process
// witness admission. Build-time paths are disposable CI fixture paths only.
// This is NOT publisher/installer OS attestation or malicious same-user security.

use crate::sync::filegate_lock_probe::{PinnedProbeRoot, LOCK_NAME};
use crate::sync::host_custody_registry::{
    HostCustodyRecord, PhysicalObjectId, SharedHostCustodyRegistry,
};
use crate::sync::host_install_witness::create_test_witness_once;
use crate::sync::VaultId;

const WITNESS_CHILD: &str =
    "app_core::installed_host_tests::witness_installed_core_child";

#[test]
fn witness_installed_core_child() {
    if std::env::var_os("AKULA_B6B_CHILD_MODE").is_none() {
        return;
    }
    let mode = std::env::var("AKULA_B6B_CHILD_MODE").unwrap();
    let appdata = PathBuf::from(std::env::var_os("AKULA_B6B_APPDATA").unwrap());
    let root = PathBuf::from(std::env::var_os("AKULA_B6B_ROOT").unwrap());
    let report = PathBuf::from(std::env::var_os("AKULA_B6B_REPORT").unwrap());
    let core = Core::open(&appdata, events());
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    let note = root.join("Shared.md");
    let accepted = match mode.as_str() {
        "positive" => gate::write(
            &core, &note, "second process committed", None, Source::Me, None,
        ).is_ok(),
        "hold" => gate::create(
            &core, &root.join("Forbidden.md"), "must not land", Source::Me, None,
        ).is_ok(),
        _ => panic!("invalid child test action"),
    };
    fs::write(&report, if accepted { "admitted" } else { "hold" }).unwrap();
}

#[test]
fn compiler_pinned_witness_survives_cold_restart_and_refuses_foreign_empty_ledger() {
    // Never create a non-CI installation witness at arbitrary build paths.
    if std::env::var_os("AKULA_B6B_TEST_ALLOW_PROVISION").is_none() {
        return;
    }
    let db = PathBuf::from(option_env!("AKULA_INSTALLED_HOST_CUSTODY_DB")
        .expect("dedicated CI requires compiled database path"));
    let witness = PathBuf::from(option_env!("AKULA_INSTALLED_HOST_WITNESS_PATH")
        .expect("dedicated CI requires compiled independent witness path"));
    assert!(db.is_absolute() && witness.is_absolute());
    assert_eq!(db.file_name().and_then(|x| x.to_str()),
        Some("akula-ci-b6b-ledger.sqlite3"));
    assert_eq!(witness.file_name().and_then(|x| x.to_str()),
        Some("akula-ci-b6b-receipt.json"));
    assert_ne!(db.parent(), witness.parent());
    fs::create_dir_all(db.parent().unwrap()).unwrap();
    fs::create_dir_all(witness.parent().unwrap()).unwrap();
    assert!(!db.exists() && !witness.exists(),
        "CI must not adopt or reset a preexisting host installation");

    let temp = tempfile::tempdir().unwrap();
    let clean = temp.path().join("clean-vault");
    let protected = temp.path().join("protected-vault");
    let a = temp.path().join("appdata-A");
    let b = temp.path().join("appdata-B");
    let c = temp.path().join("appdata-C");
    let custody = temp.path().join("held-custody");
    for dir in [&clean, &protected, &a, &b, &c, &custody] {
        fs::create_dir(dir).unwrap();
    }
    let clean = fs::canonicalize(clean).unwrap();
    let protected = fs::canonicalize(protected).unwrap();
    let mut host = SharedHostCustodyRegistry::provision_new(
        &db, &[clean.as_path(), protected.as_path(), a.as_path(), b.as_path()]
    ).unwrap();
    drop(host);
    create_test_witness_once(&db, &witness).unwrap();
    let original_note = protected.join("Original.md");
    fs::write(&original_note, "protected original").unwrap();

    let first = Core::open(&a, events());
    assert!(first.requires_installed_host_epoch());
    first.ui_state.resolve_workspace(clean.to_str().unwrap(), 1).unwrap();
    first.ui_state.resolve_workspace(protected.to_str().unwrap(), 2).unwrap();
    let shared = clean.join("Shared.md");
    gate::create(&first, &shared, "first version", Source::Me, None)
        .expect("valid installed witness must admit unenrolled create");
    assert_eq!(fs::read_to_string(&shared).unwrap(), "first version");
    assert!(gate::create_binary(&first, &clean.join("Forbid.bin"), b"x").is_err());

    // Only the already-provisioned shared host can enroll a protected root.
    fs::write(custody.join(LOCK_NAME), b"").unwrap();
    let held = PinnedProbeRoot::open(&custody).unwrap()
        .try_lock().unwrap().expect("exclusive custody fixture");
    let physical = PinnedProbeRoot::open(&protected).unwrap().identity();
    host = SharedHostCustodyRegistry::open_existing(&db).unwrap();
    host.enroll(&HostCustodyRecord {
        root_path: protected.to_string_lossy().into_owned(),
        root_identity: PhysicalObjectId { device: physical.0, file: physical.1 },
        vault_id: VaultId::new(),
        profile_id: "approved-s3".into(),
        custody_scope_path: fs::canonicalize(&custody).unwrap()
            .to_string_lossy().into_owned(),
        lock_identity: PhysicalObjectId {
            device: held.identity().0, file: held.identity().1,
        },
    }).unwrap();
    drop(host);
    drop(first);

    let report = temp.path().join("child-report.txt");
    let forged_runtime_witness = temp.path().join("runtime-forgery.json");
    fs::write(&forged_runtime_witness, b"not the compiler-pinned witness").unwrap();
    let child = |mode: &str, root: &Path| {
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact").arg(WITNESS_CHILD).arg("--nocapture")
            .env("AKULA_B6B_CHILD_MODE", mode)
            .env("AKULA_B6B_APPDATA", &b)
            .env("AKULA_B6B_ROOT", root)
            .env("AKULA_B6B_REPORT", &report)
            .env("AKULA_INSTALLED_HOST_WITNESS_PATH", &forged_runtime_witness)
            .env("AKULA_INSTALLED_HOST_CUSTODY_DB",
                temp.path().join("forged-empty.sqlite3"))
            .output().unwrap();
        assert!(output.status.success(),
            "child mode={mode}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stdout)
            .contains("witness_installed_core_child ... ok"),
            "child entrypoint did not execute: {}", String::from_utf8_lossy(&output.stdout));
        fs::read_to_string(&report).unwrap()
    };
    assert_eq!(child("positive", &clean), "admitted");
    assert_eq!(fs::read_to_string(&shared).unwrap(), "second process committed");
    assert_eq!(child("hold", &protected), "hold");
    assert_eq!(fs::read_to_string(&original_note).unwrap(), "protected original");
    assert!(!protected.join("Forbidden.md").exists());

    // No active Core/registry connection retains the original DB. A genuine,
    // schema-valid empty replacement must NOT reset earlier S3 enrollment.
    let moved = db.with_file_name("original-ledger-preserved.sqlite3");
    let alternate = db.with_file_name("foreign-valid-empty.sqlite3");
    fs::rename(&db, &moved).unwrap();
    drop(SharedHostCustodyRegistry::provision_new(
        &db, &[clean.as_path(), protected.as_path(), a.as_path()]
    ).unwrap());
    assert_eq!(SharedHostCustodyRegistry::open_existing(&db).unwrap()
        .inspect(&protected, PhysicalObjectId {
            device: physical.0, file: physical.1
        }).unwrap(), None, "unwitnessed code would falsely clear enrollment");
    assert_eq!(child("hold", &clean), "hold");
    let after_swap = Core::open(&c, events());
    after_swap.ui_state.resolve_workspace(clean.to_str().unwrap(), 1).unwrap();
    assert!(gate::create(&after_swap, &clean.join("Reset.md"),
        "must not land", Source::Me, None).is_err());
    assert!(!clean.join("Reset.md").exists());
    drop(after_swap);
    fs::rename(&db, &alternate).unwrap();
    fs::rename(&moved, &db).unwrap();

    // Removal of the witness must also HOLD. Nothing reissues it from the
    // restored (now perfectly valid) DB or from alternate appdata.
    let parked_witness = witness.with_file_name("witness-preserved.json");
    fs::rename(&witness, &parked_witness).unwrap();
    assert_eq!(child("hold", &clean), "hold");
    fs::rename(&parked_witness, &witness).unwrap();
    assert_eq!(child("positive", &clean), "admitted");
    assert_eq!(fs::read_to_string(&original_note).unwrap(), "protected original");
    assert!(!clean.join("Forbidden.md").exists());
    drop(held);
}
