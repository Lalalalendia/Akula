use super::*;
use super::super::{
    anchor_store::{AnchorStore, AnchorStoreError},
    state::{StudyStore, StateError},
    store::{RegistryStore, StoreError},
};

const CREATE: &str = "CREATE TABLE metadata (
    id INTEGER PRIMARY KEY CHECK(id=1),
    revision INTEGER NOT NULL CHECK(revision>=0),
    payload BLOB NOT NULL
);";
const INSERT: &str = "INSERT INTO metadata(id,revision,payload) VALUES(1,0,?1)";

fn seed() -> Result<Vec<u8>, &'static str> {
    Ok(b"initial".to_vec())
}

fn version(connection: &Connection) -> i64 {
    connection.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap()
}

fn table_count(connection: &Connection) -> i64 {
    connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        [], |row| row.get(0),
    ).unwrap()
}

#[test]
fn configured_connection_and_existing_payload_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("metadata.sqlite");
    let db = open(&path, 1, CREATE, INSERT, seed).unwrap();
    for (name, expected) in [("foreign_keys", 1_i64), ("synchronous", 2), ("busy_timeout", 3000)] {
        let actual: i64 = db.pragma_query_value(None, name, |row| row.get(0)).unwrap();
        assert_eq!(actual, expected, "{name}");
    }
    let mode: String = db.pragma_query_value(None, "journal_mode", |row| row.get(0)).unwrap();
    assert_eq!(mode, "wal");
    assert_eq!(version(&db), 1);
    db.execute("UPDATE metadata SET revision=7, payload=?1 WHERE id=1", params![&b"saved"[..]]).unwrap();
    drop(db);
    let db = open(&path, 1, CREATE, INSERT, || -> Result<Vec<u8>, &'static str> {
        panic!("existing DB must not regenerate payload")
    }).unwrap();
    let saved: (i64, Vec<u8>) = db.query_row(
        "SELECT revision,payload FROM metadata WHERE id=1", [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(saved, (7, b"saved".to_vec()));
}

#[test]
fn foreign_and_future_schema_remain_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("foreign.sqlite");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE unrelated(value TEXT); INSERT INTO unrelated VALUES('keep');").unwrap();
    drop(db);
    for expected_version in [0, 99] {
        let db = Connection::open(&path).unwrap();
        db.pragma_update(None, "user_version", expected_version).unwrap();
        drop(db);
        assert!(matches!(open(&path, 1, CREATE, INSERT, seed), Err(OpenError::UnsupportedSchema)));
        let db = Connection::open(&path).unwrap();
        assert_eq!(version(&db), expected_version);
        assert_eq!(table_count(&db), 1);
        let saved: String = db.query_row("SELECT value FROM unrelated", [], |row| row.get(0)).unwrap();
        assert_eq!(saved, "keep");
    }
}

#[test]
fn failed_payload_or_insert_does_not_commit_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollback.sqlite");
    assert!(matches!(open(&path, 1, CREATE, INSERT, || Err::<Vec<u8>, _>("payload")), Err(OpenError::Payload("payload"))));
    assert!(matches!(open(&path, 1, CREATE,
        "INSERT INTO metadata(id,revision,payload) VALUES(2,0,?1)", seed), Err(OpenError::Storage)));
    let db = Connection::open(&path).unwrap();
    assert_eq!(version(&db), 0);
    assert_eq!(table_count(&db), 0);
    drop(db);
    assert_eq!(version(&open(&path, 1, CREATE, INSERT, seed).unwrap()), 1);
    assert!(matches!(open(&dir.path().join("missing").join("db.sqlite"), 1, CREATE, INSERT, seed),
        Err(OpenError::Storage)));
}

#[test]
fn each_real_store_reopens_legacy_v1_bytes_and_keeps_its_own_schema() {
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        ("language_anchors", br#"{"version":1,"notes":[]}"#.as_slice()),
        ("language_study_state", br#"{"version":1,"state":{"registry":{"collections":[],"tables":[]},"anchors":[]}}"#.as_slice()),
        ("language_registry", br#"{"version":1,"registry":{"collections":[],"tables":[]}}"#.as_slice()),
    ];
    for (index, (table, payload)) in cases.iter().enumerate() {
        let path = dir.path().join(format!("{table}.sqlite"));
        // Independent pre-refactor format fixture, with a nonzero revision.
        let db = Connection::open(&path).unwrap();
        db.execute_batch(&format!(
            "CREATE TABLE {table} (
                id INTEGER PRIMARY KEY CHECK(id=1), revision INTEGER NOT NULL CHECK(revision>=0),
                payload BLOB NOT NULL
             ); PRAGMA user_version=1;"
        )).unwrap();
        db.execute(&format!("INSERT INTO {table}(id,revision,payload) VALUES(1,7,?1)"), params![payload]).unwrap();
        drop(db);
        match index {
            0 => assert_eq!(AnchorStore::open(&path).unwrap().load().unwrap().revision, 7),
            1 => assert_eq!(StudyStore::open(&path).unwrap().load().unwrap().revision, 7),
            2 => assert_eq!(RegistryStore::open(&path).unwrap().load().unwrap().revision, 7),
            _ => unreachable!(),
        }
        // A different store must not adopt or overwrite this database.
        if index != 0 {
            assert!(matches!(AnchorStore::open(&path), Err(AnchorStoreError::CorruptStore)));
        }
        if index != 1 {
            assert!(matches!(StudyStore::open(&path), Err(StateError::Corrupt)));
        }
        if index != 2 {
            assert!(matches!(RegistryStore::open(&path), Err(StoreError::CorruptStore)));
        }
        let db = Connection::open(&path).unwrap();
        assert_eq!(version(&db), 1);
        assert_eq!(table_count(&db), 1);
        let saved: (i64, Vec<u8>) = db.query_row(
            &format!("SELECT revision,payload FROM {table} WHERE id=1"),
            [], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(saved.0, 7);
        assert_eq!(saved.1.as_slice(), *payload);
    }
}

fn verify_full_replacement_guards<S, E: std::fmt::Debug + PartialEq + Clone>(
    table: &str, future_payload: &[u8],
    open_store: impl Fn(&Path) -> S,
    save: impl Fn(&mut S, u64) -> Result<u64, E>,
    errors: [E; 5],
) {
    let [stale, corrupt, future, exhausted, storage] = errors;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replace.sqlite");
    let mut store = open_store(&path);
    let mut second = open_store(&path);
    let db = Connection::open(&path).unwrap();
    let read_sql = format!("SELECT revision,payload FROM {table} WHERE id=1");
    let read = || -> (i64, Vec<u8>) {
        db.query_row(&read_sql, [], |row| Ok((row.get(0)?, row.get(1)?))).unwrap()
    };
    let update_sql = format!("UPDATE {table} SET revision=?1,payload=?2 WHERE id=1");
    let valid = read().1;
    let set = |revision: i64, payload: &[u8]| {
        db.execute(&update_sql, params![revision, payload]).unwrap();
    };

    // Full replacement increments even for identical valid payloads.
    assert_eq!(save(&mut store, 0), Ok(1));
    assert_eq!(save(&mut second, 0), Err(stale.clone()));
    assert_eq!(read(), (1, valid.clone()));
    assert_eq!(save(&mut second, 1), Ok(2));
    assert_eq!(read(), (2, valid.clone()));

    // Incumbent decoding precedes revision comparison; never repair corruption.
    set(2, future_payload);
    assert_eq!(save(&mut store, 0), Err(future));
    assert_eq!(read(), (2, future_payload.to_vec()));
    set(2, b"not-json");
    assert_eq!(save(&mut store, 0), Err(corrupt.clone()));
    assert_eq!(read(), (2, b"not-json".to_vec()));

    set(i64::MAX, &valid);
    assert_eq!(save(&mut store, 0), Err(stale.clone()));
    assert_eq!(save(&mut store, i64::MAX as u64), Err(exhausted));
    assert_eq!(read(), (i64::MAX, valid.clone()));
    db.execute_batch("PRAGMA ignore_check_constraints=ON;").unwrap();
    set(-1, &valid);
    db.execute_batch("PRAGMA ignore_check_constraints=OFF;").unwrap();
    assert_eq!(save(&mut store, 0), Err(corrupt.clone()));
    assert_eq!(read(), (-1, valid.clone()));

    set(2, &valid);
    // Force the two distinct SQL failure paths without changing the incumbent.
    db.execute_batch(&format!(
        "CREATE TRIGGER block_write BEFORE UPDATE ON {table}
         BEGIN SELECT RAISE(IGNORE); END;"
    )).unwrap();
    assert_eq!(save(&mut store, 2), Err(stale));
    assert_eq!(read(), (2, valid.clone()));
    db.execute_batch(&format!(
        "DROP TRIGGER block_write;
         CREATE TRIGGER block_write BEFORE UPDATE ON {table}
         BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;"
    )).unwrap();
    assert_eq!(save(&mut store, 2), Err(storage));
    assert_eq!(read(), (2, valid.clone()));
    db.execute_batch("DROP TRIGGER block_write;").unwrap();
    // Failed transactions must release the write lock.
    assert_eq!(save(&mut second, 2), Ok(3));
    assert_eq!(read(), (3, valid));

    db.execute(&format!("DELETE FROM {table} WHERE id=1"), []).unwrap();
    assert_eq!(save(&mut store, 3), Err(corrupt));
    let count: i64 = db.query_row(
        &format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn anchor_full_replacement_preserves_revision_guards_and_failure_order() {
    use AnchorStoreError::*;
    verify_full_replacement_guards(
        "language_anchors", br#"{"version":2,"notes":[]}"#,
        |path| AnchorStore::open(path).unwrap(),
        |store, revision| store.save(revision, &[]),
        [StaleRevision, CorruptStore, UnsupportedSchema, RevisionExhausted, Storage],
    );
}

#[test]
fn registry_full_replacement_preserves_revision_guards_and_failure_order() {
    use StoreError::*;
    let empty = super::super::collections::Registry { collections: vec![], tables: vec![] };
    verify_full_replacement_guards(
        "language_registry", br#"{"version":2,"registry":{"collections":[],"tables":[]}}"#,
        |path| RegistryStore::open(path).unwrap(),
        |store, revision| store.save(revision, &empty),
        [StaleRevision, CorruptStore, UnsupportedSchema, RevisionExhausted, Storage],
    );
}
