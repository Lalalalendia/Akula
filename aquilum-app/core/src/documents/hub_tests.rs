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


#[test]
fn cannot_arm_durable_editor_with_active_document_buffers() {
    use crate::app_core::{Core, CoreEvent};
    use std::sync::Arc;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let data = temp.path().join("appdata");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    let root = std::fs::canonicalize(root).unwrap();
    let note = root.join("Open.md");
    std::fs::write(&note, b"existing").unwrap();
    let core = Core::open(&data, Arc::new(|_: CoreEvent| {}));
    core.documents.open(&core, &note, 0).unwrap();
    assert!(core.documents.arm_durable_intents_for_root(&root).is_err(),
        "arming cannot race an unsubmitted CRDT editor buffer");
    core.documents.release(&core, &note);
    core.documents.arm_durable_intents_for_root(&root).unwrap();
}

#[test]
fn armed_document_write_crash_intent_blocks_reopen_before_automatic_reconcile() {
    use crate::app_core::{Core, CoreEvent};
    use crate::documents::session::{SessionIo, SessionError};
    use std::sync::Arc;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let data = temp.path().join("appdata");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    let root = std::fs::canonicalize(&root).unwrap();
    let note = root.join("Protected.md");
    std::fs::write(&note, b"original").unwrap();
    let original = blake3::hash(b"original").to_hex().to_string();
    let proposed = blake3::hash(b"published-but-unconfirmed").to_hex().to_string();
    let key = identity(&note);
    {
        let core = Core::open(&data, Arc::new(|_: CoreEvent| {}));
        let io = core.documents.io(&core, &note, &key);
        // Ordinary editor roots incur no prewrite WAL checkpoint.
        assert!(!io.begin_guarded_write("ordinary change", &original).unwrap());
        assert!(io.unresolved_native_write().unwrap().is_none());

        core.documents.arm_durable_intents_for_root(&root).unwrap();
        assert!(io.begin_guarded_write("published-but-unconfirmed", &original).unwrap());
        assert!(io.unresolved_native_write().unwrap().is_some());
        // Native File Gate could publish bytes just before the process
        // terminates, leaving no in-memory NeedsReview to recover.
        std::fs::write(&note, b"published-but-unconfirmed").unwrap();
        // SQLite still has the exact pre-commit intent, not an ACK.
    }
    let core = Core::open(&data, Arc::new(|_: CoreEvent| {}));
    let result = core.documents.open(&core, &note, 0);
    assert!(matches!(result, Err(HubError::Session(SessionError::Disk {
        code: "needs_review", ..
    }))), "reopen bypassed durable write HOLD");
    assert_eq!(std::fs::read(&note).unwrap(), b"published-but-unconfirmed");
    let store = DocumentStore::open(&data.join("documents.sqlite3")).unwrap();
    assert_eq!(store.write_intent(&key).unwrap().unwrap().proposed_hash, proposed);
    // An unchanged or matching physical hash is not clearance; the next
    // open is STILL held. No automatic confirmation or retry occurred.
    assert!(matches!(
        core.documents.open(&core, &note, 0),
        Err(HubError::Session(SessionError::Disk { code: "needs_review", .. }))
    ));
    assert_eq!(store.sync_point(&key), SyncPointLookup::Absent);
}


#[test]
fn participant_enrollment_refuses_ordinary_file_gate_mutations_after_core_restart() {
    use crate::app_core::{Core, CoreEvent};
    use crate::files::gate;
    use crate::history::Source;
    use std::sync::Arc;

    let temp = tempfile::tempdir().unwrap();
    let appdata = temp.path().join("appdata");
    let root = temp.path().join("notes");
    let other_root = temp.path().join("another-vault");
    for path in [&appdata, &root, &other_root] {
        std::fs::create_dir_all(path).unwrap();
    }
    let root = std::fs::canonicalize(&root).unwrap();
    let note = root.join("Original.md");
    std::fs::write(&note, b"preserve").unwrap();
    {
        let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
        assert!(!core.documents.requires_guarded_participant_hold());
        core.documents.enroll_guarded_participant(
            &root, "vault-not-local-uuid",
            &appdata.join("profiles.sqlite"),
            &appdata.join("custody.sqlite"),
            &temp.path().join("external-scope"),
        ).unwrap();
        assert!(core.documents.requires_guarded_participant_hold());
        assert!(gate::write(&core, &note, "unapproved", None, Source::Me, None).is_err());
        assert_eq!(std::fs::read(&note).unwrap(), b"preserve");
    }
    let second = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
    assert!(second.documents.requires_guarded_participant_hold(),
        "cold Core restart must never discard participant enrollment");
    let denied = gate::write(&second, &note, "unapproved", None, Source::Me, None);
    assert!(denied.is_err(), "cold Core re-entered ordinary File Gate");
    assert_eq!(std::fs::read(&note).unwrap(), b"preserve");
    let created = root.join("ShouldNotExist.md");
    assert!(gate::create(&second, &created, "no", Source::Me, None).is_err());
    assert!(!created.exists());
    // Until path-scoped isolation and all File Gate mutation paths are proved,
    // even an unrelated root in the SAME Core remains on conservative HOLD.
    let unrelated = other_root.join("AlsoHeld.md");
    assert!(gate::create(&second, &unrelated, "no", Source::Me, None).is_err());
    assert!(!unrelated.exists());
    assert!(second.empty_trash_guarded(&root).is_err());
}

#[test]
fn unreadable_documents_sqlite_never_downgrades_participant_policy_to_empty() {
    use crate::app_core::{Core, CoreEvent};
    use crate::files::gate;
    use crate::history::Source;
    use std::sync::Arc;

    let temp = tempfile::tempdir().unwrap();
    let appdata = temp.path().join("appdata");
    let root = temp.path().join("root");
    std::fs::create_dir_all(&appdata).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    let note = root.join("Keep.md");
    std::fs::write(&note, b"keep").unwrap();
    {
        let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
        core.documents.enroll_guarded_participant(
            &root, "vault-id", &appdata.join("profiles.sqlite"),
            &appdata.join("custody.sqlite"), &temp.path().join("scope"),
        ).unwrap();
    }
    // Simulate a corrupt app-data database after shutdown. Recovery must
    // inspect evidence, never open with an empty protection list.
    std::fs::write(appdata.join("documents.sqlite3"), b"corrupt sqlite").unwrap();
    let reopened = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
    assert!(reopened.documents.requires_guarded_participant_hold());
    assert!(gate::write(&reopened, &note, "overwrite", None, Source::Me, None).is_err());
    assert_eq!(std::fs::read(&note).unwrap(), b"keep");
}
