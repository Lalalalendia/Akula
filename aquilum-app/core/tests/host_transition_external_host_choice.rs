#![cfg(feature = "host-custody")]
//! External-crate integration test: aquilum-core is NOT compiled with cfg(test).
//! The explicit constructor may use a caller-provisioned host fixture in
//! legacy host-custody mode, but transition-selected non-test builds must HOLD
//! even if a caller hands it a valid-looking v2 installer database.
//!
//! This is a disposable test, not proof of a real OS-owned installer broker.
use aquilum_core::app_core::{Core, CoreEvent, EventSink};
use aquilum_core::files::gate;
use aquilum_core::history::Source;
use rusqlite::{Connection, OpenFlags};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn provision_well_formed_foreign_host(path: &Path, version: i64) {
    let file = std::fs::OpenOptions::new()
        .write(true).create_new(true).open(path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600)).unwrap();
    }
    drop(file);
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=FULL;
         BEGIN IMMEDIATE;
         CREATE TABLE shared_custody (
            root_key TEXT PRIMARY KEY NOT NULL,
            root_path TEXT NOT NULL UNIQUE,
            record_json BLOB NOT NULL,
            record_blake3 TEXT NOT NULL
         );"
    ).unwrap();
    if version == 2 {
        conn.execute_batch(
            "CREATE TABLE workspace_transitions (
                operation_id TEXT NOT NULL PRIMARY KEY,
                workspace_id TEXT NOT NULL UNIQUE,
                source_path TEXT NOT NULL,
                destination_path TEXT NOT NULL,
                root_key TEXT NOT NULL UNIQUE,
                record_json BLOB NOT NULL,
                record_blake3 TEXT NOT NULL
            );"
        ).unwrap();
    }
    conn.execute_batch(&format!("PRAGMA user_version = {version}; COMMIT;"))
        .unwrap();
    drop(conn);
}

#[test]
fn externally_compiled_core_cannot_substitute_a_valid_foreign_host_path() {
    let tmp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(tmp.path()).unwrap();
    let root = base.join("Knowledge");
    let app = base.join("appdata");
    let host = base.join("alternate-host");
    for path in [&root, &app, &host] {
        fs::create_dir(path).unwrap();
    }
    let foreign = host.join("custody.sqlite");
    let version = if cfg!(feature = "host-custody-transition-lab") { 2 } else { 1 };
    provision_well_formed_foreign_host(&foreign, version);

    let events: Arc<dyn EventSink> = Arc::new(|_: CoreEvent| {});
    let core = Core::open_with_installed_host(&app, &foreign, events);
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    let note = root.join("new.md");
    let result = gate::create(&core, &note, "not from installer", Source::Me, None);

    #[cfg(feature = "host-custody-transition-lab")]
    {
        assert!(result.is_err(),
            "transition lab must not accept an untrusted caller-specified host DB");
        assert!(!note.exists(), "no note may be written by foreign-host substitution");
    }
    #[cfg(not(feature = "host-custody-transition-lab"))]
    {
        // Proves the fixture is genuinely operational in the pre-existing
        // explicit legacy constructor, so a HOLD in the new mode is not
        // accidentally explained by a malformed SQLite fixture.
        assert!(result.is_ok(), "legacy fixture must be valid: {result:?}");
        assert_eq!(fs::read(&note).unwrap(), b"not from installer");
    }
}
