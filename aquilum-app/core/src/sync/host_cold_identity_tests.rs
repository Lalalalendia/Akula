// Test-only fixture for P0-B6a cold-open identity witnesses.
//
// Every expected object ID comes from the ORIGINAL already-provisioned
// registry, before the competing physical SQLite database is installed.
// Never infer expected IDs from the candidate database at cold startup.
// Tests exercise OS processes, and do not turn the caller's environment into
// a production installer trust anchor.

#[test]
fn cold_installer_witness_child_probe() {
    let Some(path) = std::env::var_os("AKULA_B6A_COLD_DB") else {
        return;
    };
    let claim = std::env::var("AKULA_B6A_COLD_EXPECTED").unwrap();
    let expected: HostLedgerObjectIds = serde_json::from_str(&claim).unwrap();
    let mode = std::env::var("AKULA_B6A_COLD_RESULT").unwrap();
    let result = SharedHostCustodyRegistry::open_existing_with_expected(
        Path::new(&path), expected,
    );
    match mode.as_str() {
        "admit-original" => {
            let registry = result.expect("original ledger must survive cold restart");
            let root = std::path::PathBuf::from(
                std::env::var_os("AKULA_B6A_COLD_ROOT").unwrap()
            );
            let found = registry.inspect(
                &root, PhysicalObjectId { device: 1001, file: 9001 },
            ).unwrap().expect("original enrollment must still exist");
            assert_eq!(found.profile_id, "approved-s3");
        }
        "hold-replaced" => {
            assert!(matches!(result, Err(HostCustodyError::InvalidHostLocation)),
                "valid-looking foreign ledger must fail supplied cold identity");
        }
        other => panic!("unexpected cold witness child mode: {other}"),
    }
}

#[test]
fn real_cold_process_rejects_a_valid_empty_database_replacement_with_external_witness() {
    const CHILD: &str =
        "sync::host_custody_registry::tests::cold_installer_witness_child_probe";
    use std::process::Command;

    let (temp, db, vault, appdata, record) = specimen();
    let mut installer = SharedHostCustodyRegistry::provision_new(
        &db, &[vault.as_path(), appdata.as_path()],
    ).unwrap();
    installer.enroll(&record).unwrap();
    let expected = installer.identity.object_ids();
    drop(installer);

    // A real OS child receives the original fixture claim. A production
    // install MUST supply it from OS-protected independent state.
    let run_child = |mode: &str| {
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact").arg(CHILD).arg("--nocapture")
            .env("AKULA_B6A_COLD_DB", &db)
            .env("AKULA_B6A_COLD_ROOT", &record.root_path)
            .env("AKULA_B6A_COLD_EXPECTED",
                 serde_json::to_string(&expected).unwrap())
            .env("AKULA_B6A_COLD_RESULT", mode)
            .output().unwrap();
        assert!(output.status.success(),
            "cold process mode={mode} failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr));
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains("cold_installer_witness_child_probe ... ok"),
            "child did not execute the intended cold check: {}",
            String::from_utf8_lossy(&output.stdout),
        );
    };

    run_child("admit-original");

    // Preserve the original inode and enrollment instead of deleting them.
    // Replace the compiler-pinned filename by a legitimate, schema-valid,
    // EMPTY SQLite ledger after the first OS process has stopped.
    let foreign = temp.path().join("valid-empty-foreign.sqlite");
    drop(SharedHostCustodyRegistry::provision_new(
        &foreign, &[vault.as_path(), appdata.as_path()],
    ).unwrap());
    let preserved = temp.path().join("original-host-ledger-kept.sqlite");
    fs::rename(&db, &preserved).unwrap();
    fs::rename(&foreign, &db).unwrap();

    run_child("hold-replaced");

    // Demonstrate why ordinary unanchored Core opening is not a cold
    // recovery proof: a valid EMPTY foreign DB appears "unenrolled".
    let unanchored = SharedHostCustodyRegistry::open_existing(&db).unwrap();
    assert_eq!(unanchored.inspect(&vault, record.root_identity).unwrap(), None);
    drop(unanchored);

    fs::rename(&db, &foreign).unwrap();
    fs::rename(&preserved, &db).unwrap();
    run_child("admit-original");
}

#[test]
fn cold_installer_witness_rejects_real_parent_directory_replacement() {
    let (temp, db, vault, appdata, record) = specimen();
    let mut installer = SharedHostCustodyRegistry::provision_new(
        &db, &[vault.as_path(), appdata.as_path()],
    ).unwrap();
    installer.enroll(&record).unwrap();
    let expected = installer.identity.object_ids();
    drop(installer);

    // Replace the original parent with a REAL directory, not a symlink
    // or junction. Existing alias checks do not detect this cold swap.
    let parent = db.parent().unwrap().to_path_buf();
    let parked = temp.path().join("preserved-original-parent");
    fs::rename(&parent, &parked).unwrap();
    fs::create_dir(&parent).unwrap();
    drop(SharedHostCustodyRegistry::provision_new(
        &db, &[vault.as_path(), appdata.as_path()],
    ).unwrap());

    assert!(matches!(
        SharedHostCustodyRegistry::open_existing_with_expected(&db, expected),
        Err(HostCustodyError::InvalidHostLocation),
    ), "cold parent substitution must fail against earlier installer IDs");
    let unanchored = SharedHostCustodyRegistry::open_existing(&db).unwrap();
    assert_eq!(unanchored.inspect(&vault, record.root_identity).unwrap(), None);
    drop(unanchored);

    // The original enrollment remains available for forensic recovery.
    let old_db = parked.join("approved.sqlite");
    let original = SharedHostCustodyRegistry::open_existing(&old_db).unwrap();
    assert_eq!(original.inspect(&vault, record.root_identity).unwrap(),
        Some(record));
}
