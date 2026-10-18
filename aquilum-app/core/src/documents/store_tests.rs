use super::*;

fn point(file: &str, text: &str) -> SyncPoint {
    SyncPoint { file_hash: file.into(), text_hash: text.into() }
}

#[test]
fn returns_the_snapshot_then_updates_in_write_order() {
    let mut store = DocumentStore::memory().expect("store");
    store.compact("a", b"state").expect("compact");
    store.append("a", b"one").expect("append");
    store.append("a", b"two").expect("append");
    store.append("b", b"other").expect("append");
    assert_eq!(store.load("a").expect("load"), vec![b"state".to_vec(), b"one".to_vec(), b"two".to_vec()]);
}

#[test]
fn compaction_replaces_the_snapshot_and_drops_folded_updates() {
    let mut store = DocumentStore::memory().expect("store");
    store.append("a", b"one").expect("append");
    store.compact("a", b"folded").expect("compact");
    assert_eq!(store.load("a").expect("load"), vec![b"folded".to_vec()]);
}

#[test]
fn knows_whether_a_document_already_has_a_replica() {
    let mut store = DocumentStore::memory().expect("store");
    assert!(!store.has_replica("a").expect("query"));
    store.append("a", b"one").expect("append");
    assert!(store.has_replica("a").expect("query"));
    store.compact("b", b"state").expect("compact");
    assert!(store.has_replica("b").expect("query"));
}

#[test]
fn an_unknown_document_has_nothing_stored() {
    let store = DocumentStore::memory().expect("store");
    assert!(store.load("missing").expect("load").is_empty());
    assert_eq!(store.sync_point("missing"), SyncPointLookup::Absent);
}

#[test]
fn remembers_and_overwrites_the_sync_point() {
    let store = DocumentStore::memory().expect("store");
    store.remember_sync_point("a", &point("f1", "t1")).expect("remember");
    store.remember_sync_point("a", &point("f2", "t2")).expect("remember");
    assert_eq!(store.sync_point("a"), SyncPointLookup::Found(point("f2", "t2")));
}

#[test]
fn forgetting_a_note_drops_its_replica_updates_and_sync_point() {
    let mut store = DocumentStore::memory().expect("store");
    store.compact("a", b"state").expect("compact");
    store.append("a", b"one").expect("append");
    store.remember_sync_point("a", &point("f", "t")).expect("remember");
    store.forget("a").expect("forget");
    assert!(store.load("a").expect("load").is_empty());
    assert_eq!(store.sync_point("a"), SyncPointLookup::Absent);
}

#[test]
fn relocation_moves_everything_and_replaces_a_stale_target() {
    let mut store = DocumentStore::memory().expect("store");
    store.compact("old", b"state").expect("compact");
    store.append("old", b"one").expect("append");
    store.remember_sync_point("old", &point("f", "t")).expect("remember");
    store.compact("new", b"stale").expect("compact");
    store.remember_sync_point("new", &point("stale", "stale")).expect("remember");

    store.relocate("old", "new").expect("relocate");

    assert!(store.load("old").expect("load").is_empty());
    assert_eq!(store.load("new").expect("load"), vec![b"state".to_vec(), b"one".to_vec()]);
    assert_eq!(store.sync_point("new"), SyncPointLookup::Found(point("f", "t")));
}

#[test]
fn refuses_a_store_written_by_a_newer_build() {
    let connection = Connection::open_in_memory().expect("database");
    connection.execute_batch("PRAGMA user_version = 99;").expect("version");
    assert!(matches!(DocumentStore::prepare(connection), Err(StoreError::UnsupportedSchema(99))));
}

#[test]
fn reopening_keeps_what_was_written() {
    let directory = std::env::temp_dir().join(format!("aquilum-store-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("directory");
    let path = directory.join("documents.sqlite3");
    {
        let store = DocumentStore::open(&path).expect("open");
        store.append("a", b"one").expect("append");
    }
    let store = DocumentStore::open(&path).expect("reopen");
    assert_eq!(store.load("a").expect("load"), vec![b"one".to_vec()]);
    drop(store);
    let _ = std::fs::remove_dir_all(&directory);
}


#[test]
fn migrates_v1_document_store_and_keeps_existing_replica_data() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("documents.sqlite3");
    {
        let old = Connection::open(&path).unwrap();
        old.execute_batch(
            "CREATE TABLE replicas (key TEXT PRIMARY KEY, state BLOB NOT NULL);
             CREATE TABLE replica_updates (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                key TEXT NOT NULL,
                data BLOB NOT NULL
             );
             CREATE INDEX replica_updates_key ON replica_updates (key, seq);
             CREATE TABLE sync_points (
                key TEXT PRIMARY KEY, file_hash TEXT NOT NULL, text_hash TEXT NOT NULL
             );
             INSERT INTO replicas VALUES ('legacy', X'010203');
             PRAGMA user_version = 1;"
        ).unwrap();
    }
    let store = DocumentStore::open(&path).unwrap();
    assert_eq!(store.load("legacy").unwrap(), vec![vec![1, 2, 3]]);
    assert!(store.write_intent("legacy").unwrap().is_none());
    assert_eq!(
        store.connection.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(),
        2
    );
}

#[test]
fn durable_intent_survives_restart_and_cannot_be_overwritten_or_silently_forgotten() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("documents.sqlite3");
    let base = blake3::hash(b"before").to_hex().to_string();
    let proposed = blake3::hash(b"after").to_hex().to_string();
    {
        let mut store = DocumentStore::open(&path).unwrap();
        store.append("note", b"unsaved CRDT edit").unwrap();
        store.remember_sync_point("note", &point(&base, &base)).unwrap();
        store.begin_write_intent("note", &base, &proposed).unwrap();
        assert!(matches!(
            store.begin_write_intent("note", &base, &proposed),
            Err(StoreError::HoldPresent)
        ));
        assert!(matches!(store.forget("note"), Err(StoreError::HoldPresent)));
        assert!(matches!(store.relocate("note", "renamed"), Err(StoreError::HoldPresent)));
    }
    {
        // Pretend the process crashed after changing primary bytes and
        // before seeing an ACK from native File Gate/HistoryService.
        let mut store = DocumentStore::open(&path).unwrap();
        assert_eq!(store.write_intent("note").unwrap(), Some(DurableWriteIntent {
            base_hash: base.clone(), proposed_hash: proposed.clone(),
        }));
        assert_eq!(store.load("note").unwrap(), vec![b"unsaved CRDT edit".to_vec()]);
        assert_eq!(store.sync_point("note"), SyncPointLookup::Found(point(&base, &base)));
        assert!(matches!(
            store.complete_write_intent("note", &base, &base),
            Err(StoreError::HoldMissing)
        ));
        assert!(matches!(store.relocate("else", "note"), Err(StoreError::HoldPresent)));
        assert!(store.write_intent("note").unwrap().is_some());
        // Only the exact successful, independently confirmed attempt may
        // retire its evidence. This is an explicit low-level test API; the
        // production editor does NOT automatically clear on startup.
        store.complete_write_intent("note", &base, &proposed).unwrap();
        assert!(store.write_intent("note").unwrap().is_none());
        store.forget("note").unwrap();
    }
}
