use super::*;
use crate::documents::resolve::SyncPointLookup;

fn note(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("aquilum-legacy-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("directory");
    let path = directory.join(format!("{name}.md"));
    std::fs::write(&path, "текст\n").expect("note");
    path
}

fn replica(path: &Path, point: Option<LegacyPoint>) -> LegacyReplica {
    LegacyReplica { path: path.to_string_lossy().into_owned(), updates: vec![b"update".to_vec()], point }
}

#[test]
fn moves_a_legacy_replica_with_its_sync_point_into_the_store() {
    let store = DocumentStore::memory().expect("store");
    let path = note("moved");
    let point = LegacyPoint { file_hash: "f".into(), text_hash: "t".into() };
    assert!(import_one(&store, &replica(&path, Some(point))));
    let key = identity(&path);
    assert_eq!(store.load(&key).expect("load"), vec![b"update".to_vec()]);
    assert_eq!(
        store.sync_point(&key),
        SyncPointLookup::Found(SyncPoint { file_hash: "f".into(), text_hash: "t".into() }),
    );
}

#[test]
fn never_overrides_a_replica_the_core_already_keeps() {
    let mut store = DocumentStore::memory().expect("store");
    let path = note("kept");
    store.compact(&identity(&path), b"current").expect("compact");
    assert!(!import_one(&store, &replica(&path, None)));
    assert_eq!(store.load(&identity(&path)).expect("load"), vec![b"current".to_vec()]);
}

#[test]
fn skips_a_note_that_no_longer_exists() {
    let store = DocumentStore::memory().expect("store");
    let path = note("gone");
    std::fs::remove_file(&path).expect("remove");
    assert!(!import_one(&store, &replica(&path, None)));
    assert!(!store.has_replica(&identity(&path)).expect("query"));
}


#[cfg(unix)]
#[test]
fn sync_reservation_blocks_canonical_aliases_in_both_directions() {
    use crate::app_core::{Core, CoreEvent};
    use std::os::unix::fs::symlink;
    use std::sync::Arc;

    let temp = tempfile::tempdir().unwrap();
    let actual_root = temp.path().join("real");
    let alias_root = temp.path().join("alias");
    std::fs::create_dir(&actual_root).unwrap();
    symlink(&actual_root, &alias_root).unwrap();
    let actual = actual_root.join("text.md");
    let alias = alias_root.join("text.md");
    std::fs::write(&actual, b"content\n").unwrap();

    let appdata = temp.path().join("appdata");
    std::fs::create_dir(&appdata).unwrap();
    let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));

    core.documents.open(&core, &alias, 0).unwrap();
    assert!(matches!(
        core.documents.reserve_sync_closed_document(&actual),
        Err(SyncAdmissionError::DocumentOpen)
    ));
    core.documents.release(&core, &alias);

    let reservation = core.documents.reserve_sync_closed_document(&actual).unwrap();
    assert!(matches!(
        core.documents.open(&core, &alias, 0),
        Err(HubError::SyncReserved)
    ));
    drop(reservation);
    core.documents.open(&core, &alias, 0).unwrap();
    core.documents.release(&core, &alias);
}

#[cfg(windows)]
#[test]
fn sync_reservation_blocks_windows_junction_alias() {
    use crate::app_core::{Core, CoreEvent};
    use std::process::Command;
    use std::sync::Arc;

    let temp = tempfile::tempdir().unwrap();
    let actual_root = temp.path().join("real");
    let alias_root = temp.path().join("alias");
    std::fs::create_dir(&actual_root).unwrap();
    let output = Command::new("cmd")
        .arg("/C").arg("mklink").arg("/J")
        .arg(&alias_root).arg(&actual_root)
        .output().unwrap();
    assert!(output.status.success(), "mklink /J: {}", String::from_utf8_lossy(&output.stderr));
    let actual = actual_root.join("text.md");
    let alias = alias_root.join("text.md");
    std::fs::write(&actual, b"content\n").unwrap();

    let appdata = temp.path().join("appdata");
    std::fs::create_dir(&appdata).unwrap();
    let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
    core.documents.open(&core, &alias, 0).unwrap();
    assert!(matches!(
        core.documents.reserve_sync_closed_document(&actual),
        Err(SyncAdmissionError::DocumentOpen)
    ));
    core.documents.release(&core, &alias);
    let reservation = core.documents.reserve_sync_closed_document(&actual).unwrap();
    assert!(matches!(core.documents.open(&core, &alias, 0), Err(HubError::SyncReserved)));
    drop(reservation);
}
