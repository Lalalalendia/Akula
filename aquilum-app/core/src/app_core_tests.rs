use super::{Core, CoreEvent, EventSink};
use crate::documents::session::SYSTEM_CLIENT;
use crate::documents::DocumentEvent;
use crate::files::gate;
use crate::history::Source;
use crate::mcp::bridge::{Disposition, Navigation};
use serde_json::json;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Recording(Mutex<Vec<CoreEvent>>);

impl EventSink for Recording {
    fn emit(&self, event: CoreEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Recording {
    fn names(&self) -> Vec<&'static str> {
        self.0.lock().unwrap().iter().map(CoreEvent::name).collect()
    }

    fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

fn open_core() -> (tempfile::TempDir, Arc<Core>, Arc<Recording>) {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    fs::create_dir_all(&data).unwrap();
    let events = Arc::new(Recording::default());
    let core = Core::open(&data, Arc::clone(&events) as Arc<dyn EventSink>);
    (directory, core, events)
}

#[test]
fn history_failure_after_primary_note_commit_uses_distinct_event_and_recovers() {
    use crate::history::service::HistoryFault;

    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    fs::create_dir_all(&data).unwrap();
    let events = Arc::new(Recording::default());
    let core = Core::open(&data, Arc::clone(&events) as Arc<dyn EventSink>);
    let vault = directory.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    core.ui_state.resolve_workspace(vault.to_str().unwrap(), 1).unwrap();

    let note = vault.join("Important.md");
    fs::write(&note, b"untouched original").unwrap();
    let expected_hash = crate::files::document::hash_bytes(b"untouched original");
    core.history.fail_once(HistoryFault::CreateVersion);
    events.clear();

    let first = gate::write(
        &core,
        &note,
        "first primary commit",
        Some(&expected_hash),
        Source::Me,
        None,
    ).unwrap();
    assert_eq!(fs::read(&note).unwrap(), b"first primary commit");
    let names = events.names();
    assert!(names.contains(&"note-history-save-failed"), "{names:?}");
    assert!(!names.contains(&"note-history-changed"), "{names:?}");
    assert!(!names.contains(&"document-save-failed"), "{names:?}");
    let recorded = events.0.lock().unwrap();
    let error = recorded.iter().find_map(|event| match event {
        CoreEvent::NoteHistorySaveFailed(error) => Some(error),
        _ => None,
    }).unwrap();
    assert_eq!(error.action, "write baseline");
    assert_eq!(error.path, note.to_string_lossy());
    drop(recorded);

    events.clear();
    let retry = gate::write(
        &core,
        &note,
        "second primary commit",
        Some(&first.hash),
        Source::Me,
        None,
    ).unwrap();
    assert_eq!(fs::read(&note).unwrap(), b"second primary commit");
    assert_ne!(retry.hash, first.hash);
    assert!(events.names().contains(&"note-history-changed"));
    assert!(!events.names().contains(&"note-history-save-failed"));
    let page = core.history.page(&|| core.known_roots(), &note, 0, 10);
    assert_eq!(page.total, 2, "the original baseline and retry should persist");
}

#[test]
fn history_move_failure_after_primary_rename_has_distinct_event_and_retry() {
    use crate::history::service::HistoryFault;

    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    fs::create_dir_all(&data).unwrap();
    let events = Arc::new(Recording::default());
    let core = Core::open(&data, Arc::clone(&events) as Arc<dyn EventSink>);
    let vault_dir = directory.path().join("vault");
    fs::create_dir_all(&vault_dir).unwrap();
    // On macOS the temp directory can be spelled /var/... or /private/var/...
    // Use the same canonical root for the stored workspace and every note.
    let vault = fs::canonicalize(&vault_dir).unwrap();
    core.ui_state.resolve_workspace(vault.to_str().unwrap(), 1).unwrap();
    let original = vault.join("Before.md");
    let moved = vault.join("After.md");
    gate::create(&core, &original, "history snapshot", Source::Agent, None).unwrap();
    assert_eq!(core.history.page(&|| core.known_roots(), &original, 0, 10).total, 1);
    events.clear();

    core.history.fail_once(HistoryFault::MoveBefore);
    gate::move_across(&core, &original, &moved).unwrap();
    assert!(!original.exists(), "primary file must really be moved");
    assert_eq!(fs::read_to_string(&moved).unwrap(), "history snapshot");
    let names = events.names();
    assert!(names.contains(&"notes-relocated"), "{names:?}");
    assert!(names.contains(&"note-history-move-failed"), "{names:?}");
    assert!(!names.contains(&"document-save-failed"), "{names:?}");
    let seen = events.0.lock().unwrap();
    let failure = seen.iter().find_map(|event| match event {
        CoreEvent::NoteHistoryMoveFailed(failure) => Some(failure),
        _ => None,
    }).unwrap();
    assert_eq!(failure.from, original.to_string_lossy());
    assert_eq!(failure.to, moved.to_string_lossy());
    assert_eq!(failure.action, "move history");
    assert!(failure.may_have_partial_files);
    drop(seen);

    assert_eq!(core.history.page(&|| core.known_roots(), &moved, 0, 10).total, 0);
    core.history.try_moved(&|| core.known_roots(), &original, &moved).unwrap();
    assert_eq!(core.history.page(&|| core.known_roots(), &moved, 0, 10).total, 1);
}

fn cross_vault_fixture() -> (tempfile::TempDir, Arc<Core>, Arc<Recording>, std::path::PathBuf, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    let root_a = directory.path().join("workspace-a");
    let root_b = directory.path().join("workspace-b");
    fs::create_dir_all(&data).unwrap();
    fs::create_dir_all(&root_a).unwrap();
    fs::create_dir_all(&root_b).unwrap();
    // Avoid macOS /var -> /private/var spelling differences in workspace
    // registration and filesystem paths used in the same regression.
    let root_a = fs::canonicalize(root_a).unwrap();
    let root_b = fs::canonicalize(root_b).unwrap();
    let events = Arc::new(Recording::default());
    let core = Core::open(&data, Arc::clone(&events) as Arc<dyn EventSink>);
    core.ui_state.resolve_workspace(root_a.to_str().unwrap(), 1).unwrap();
    core.ui_state.resolve_workspace(root_b.to_str().unwrap(), 2).unwrap();
    (directory, core, events, root_a, root_b)
}

fn history_cross_vault_write_test_hold(owner: &Path, source: &Path, destination: &Path) -> std::path::PathBuf {
    let directory = owner.join(".aquilum/history-moves");
    fs::create_dir_all(&directory).unwrap();
    let marker = directory.join("cross-vault-hold-v1.json");
    let record = json!({
        "schema": 1,
        "operation_id": "550e8400-e29b-41d4-a716-446655440000",
        "side": "source",
        "source_vault": crate::search::paths::identity(source),
        "destination_vault": crate::search::paths::identity(destination),
        "source_note": crate::search::paths::identity(&source.join("Original.md")),
        "destination_note": crate::search::paths::identity(&destination.join("Original.md")),
        "primary_blake3": blake3::hash(b"original").to_hex().to_string(),
        "history_inventory_blake3": blake3::hash(b"history").to_hex().to_string(),
    });
    fs::write(&marker, serde_json::to_vec(&record).unwrap()).unwrap();
    marker
}

#[test]
fn history_cross_vault_filegate_hold_denies_primary_mutations_and_keeps_versions() {
    let (directory, core, events, root_a, root_b) = cross_vault_fixture();
    let original = root_a.join("Original.md");
    gate::create(&core, &original, "original", Source::Agent, None).unwrap();
    let history_dir = root_a.join(".aquilum/history/Original.md");
    let mut before: Vec<_> = fs::read_dir(&history_dir).unwrap()
        .map(|entry| entry.unwrap().file_name()).collect();
    before.sort();
    assert!(!before.is_empty());
    let marker = history_cross_vault_write_test_hold(&root_a, &root_a, &root_b);
    events.clear();

    let checks = [
        gate::write(&core, &original, "destructive edit", None, Source::Agent, None).is_err(),
        gate::create(&core, &root_a.join("New.md"), "new", Source::Agent, None).is_err(),
        gate::create_binary(&core, &root_a.join("Attachment.png"), b"binary").is_err(),
        gate::copy(&core, &original, &root_a.join("Copy.md")).is_err(),
        gate::rename(&core, &original, &root_a.join("Renamed.md")).is_err(),
        gate::move_across(&core, &original, &root_a.join("Moved.md")).is_err(),
        gate::trash(&core, &root_a, &original).is_err(),
        gate::create_folder(&core, &root_a.join("Folder")).is_err(),
        gate::ensure_folder_tree(&core, &root_a.join("Templates/Deep")).is_err(),
        gate::ensure_trash(&core, &root_a).is_err(),
        core.empty_trash_guarded(&root_a).is_err(),
    ];
    assert!(checks.iter().all(|denied| *denied), "a File Gate entrypoint bypassed HOLD: {checks:?}");
    assert_eq!(fs::read(&original).unwrap(), b"original");
    let mut after: Vec<_> = fs::read_dir(&history_dir).unwrap()
        .map(|entry| entry.unwrap().file_name()).collect();
    after.sort();
    assert_eq!(after, before);
    assert!(marker.exists());
    assert!(!root_a.join("New.md").exists());
    assert!(!root_a.join("Renamed.md").exists());
    assert!(!root_a.join("Templates").exists());
    assert!(!events.names().contains(&"notes-relocated"));

    // A new Core which registered ONLY an unrelated root may still write,
    // without consulting another Core's unrelated workspace registration.
    let other_data = directory.path().join("unrelated-data");
    fs::create_dir_all(&other_data).unwrap();
    let independent = Core::open(&other_data, Arc::new(Recording::default()));
    independent.ui_state.resolve_workspace(root_b.to_str().unwrap(), 1).unwrap();
    let independent_note = root_b.join("Independent.md");
    gate::create(&independent, &independent_note, "independent", Source::Agent, None).unwrap();
    assert_eq!(fs::read(&independent_note).unwrap(), b"independent");
    assert_eq!(independent.history.page(&|| independent.known_roots(), &independent_note, 0, 10).total, 1);
}

#[test]
fn history_cross_vault_filegate_corrupt_hold_cannot_allow_primary_write_after_restart() {
    let (directory, core, _events, root_a, root_b) = cross_vault_fixture();
    let original = root_a.join("Original.md");
    gate::create(&core, &original, "original", Source::Agent, None).unwrap();
    let marker = history_cross_vault_write_test_hold(&root_a, &root_a, &root_b);
    fs::write(&marker, b"{broken").unwrap();

    let app_data = directory.path().join("restarted-data");
    fs::create_dir_all(&app_data).unwrap();
    let restarted = Core::open(&app_data, Arc::new(Recording::default()));
    restarted.ui_state.resolve_workspace(root_a.to_str().unwrap(), 1).unwrap();
    assert!(gate::write(&restarted, &original, "changed", None, Source::Agent, None).is_err());
    assert!(gate::create(&restarted, &root_a.join("Unrelated.md"), "new", Source::Agent, None).is_err());
    assert_eq!(fs::read(&original).unwrap(), b"original");
    assert!(!root_a.join("Unrelated.md").exists());
    assert_eq!(fs::read(&marker).unwrap(), b"{broken");
}

#[test]
fn history_cross_vault_primary_move_is_denied_before_any_disk_or_event_change() {
    let (directory, core, events, root_a, root_b) = cross_vault_fixture();
    let original = root_a.join("Versioned.md");
    let target = root_b.join("Versioned.md");
    gate::create(&core, &original, "valuable version", Source::Agent, None).unwrap();
    let before = core.history.page(&|| core.known_roots(), &original, 0, 10);
    assert_eq!(before.total, 1);
    events.clear();

    let err = gate::move_across(&core, &original, &target).unwrap_err();
    assert!(matches!(&err, crate::files::error::FileCommandError::Task { .. }), "{err:?}");
    assert_eq!(fs::read_to_string(&original).unwrap(), "valuable version");
    assert!(!target.exists());
    assert_eq!(core.history.page(&|| core.known_roots(), &original, 0, 10).total, 1);
    assert!(!root_a.join(".aquilum/history-moves/pending-v1.json").exists());
    assert!(!root_b.join(".aquilum/history-moves/pending-v1.json").exists());
    assert!(!events.names().contains(&"notes-relocated"));
    assert!(!events.names().contains(&"note-history-move-failed"));

    // Another Core with only the destination registered observes no
    // incoming document and therefore cannot race a half-finished transfer.
    let second_data = directory.path().join("data-b");
    fs::create_dir_all(&second_data).unwrap();
    let second = Core::open(&second_data, Arc::new(Recording::default()));
    second.ui_state.resolve_workspace(root_b.to_str().unwrap(), 1).unwrap();
    assert_eq!(second.history.page(&|| second.known_roots(), &target, 0, 10).total, 0);
}

#[test]
fn history_cross_vault_hold_destination_only_core_rejects_history_write_after_restart() {
    let temp = tempfile::tempdir().unwrap();
    let source_root = temp.path().join("source");
    let destination_root = temp.path().join("destination");
    let data_a = temp.path().join("appdata-a");
    let data_b = temp.path().join("appdata-b");
    for directory in [&source_root, &destination_root, &data_a, &data_b] {
        fs::create_dir_all(directory).unwrap();
    }
    let source_root = fs::canonicalize(source_root).unwrap();
    let destination_root = fs::canonicalize(destination_root).unwrap();
    let events = Arc::new(Recording::default());
    let source_core = Core::open(&data_a, Arc::clone(&events) as Arc<dyn EventSink>);
    source_core.ui_state.resolve_workspace(source_root.to_str().unwrap(), 1).unwrap();
    let destination_core = Core::open(&data_b, Arc::new(Recording::default()));
    destination_core.ui_state.resolve_workspace(destination_root.to_str().unwrap(), 1).unwrap();

    let source_note = source_root.join("Original.md");
    let destination_note = destination_root.join("Transferred.md");
    gate::create(&source_core, &source_note, "original", Source::Agent, None).unwrap();
    gate::create(&destination_core, &destination_note, "existing", Source::Agent, None).unwrap();
    let history_dir = destination_root.join(".aquilum/history/Transferred.md");
    let before: Vec<_> = fs::read_dir(&history_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(!before.is_empty());

    let hold_dir = destination_root.join(".aquilum/history-moves");
    fs::create_dir_all(&hold_dir).unwrap();
    let marker = hold_dir.join("cross-vault-hold-v1.json");
    let hold = json!({
        "schema": 1,
        "operation_id": "550e8400-e29b-41d4-a716-446655440000",
        "side": "destination",
        "source_vault": crate::search::paths::identity(&source_root),
        "destination_vault": crate::search::paths::identity(&destination_root),
        "source_note": crate::search::paths::identity(&source_note),
        "destination_note": crate::search::paths::identity(&destination_note),
        "primary_blake3": blake3::hash(b"original").to_hex().to_string(),
        "history_inventory_blake3": blake3::hash(b"history").to_hex().to_string(),
    });
    fs::write(&marker, serde_json::to_vec(&hold).unwrap()).unwrap();
    drop(destination_core);

    // The reopened destination-only Core has never registered source_root.
    let reopened = Core::open(&data_b, Arc::new(Recording::default()));
    reopened.ui_state.resolve_workspace(destination_root.to_str().unwrap(), 1).unwrap();
    assert!(reopened.history.try_saved(
        &|| reopened.known_roots(),
        &destination_note,
        crate::history::NoteWrite {
            disk_before: None,
            text: "unacknowledged edit",
            hash: "never-committed",
            source: Source::Agent,
            name: None,
        },
    ).is_err());
    assert_eq!(reopened.history.page(&|| reopened.known_roots(), &destination_note, 0, 10).total, 0);
    assert_eq!(fs::read(&destination_note).unwrap(), b"existing");
    let after: Vec<_> = fs::read_dir(&history_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(after, before);
    assert!(gate::move_across(&source_core, &source_note, &destination_note).is_err());
    assert_eq!(fs::read(&source_note).unwrap(), b"original");
    assert!(marker.exists(), "the marker is never cleared automatically");

    // Corruption cannot turn HOLD into permission, even after another reopen.
    fs::write(&marker, b"{invalid").unwrap();
    let third = Core::open(&data_b, Arc::new(Recording::default()));
    third.ui_state.resolve_workspace(destination_root.to_str().unwrap(), 1).unwrap();
    assert!(third.history.try_saved(
        &|| third.known_roots(),
        &destination_note,
        crate::history::NoteWrite {
            disk_before: None,
            text: "still denied",
            hash: "also-not-committed",
            source: Source::Agent,
            name: None,
        },
    ).is_err());
    assert_eq!(fs::read(&destination_note).unwrap(), b"existing");
}

#[test]
fn history_cross_vault_folder_rename_is_denied_before_primary_folder_moves() {
    let (_directory, core, events, root_a, root_b) = cross_vault_fixture();
    let old_folder = root_a.join("Folder");
    fs::create_dir(&old_folder).unwrap();
    let note = old_folder.join("Inside.md");
    gate::create(&core, &note, "version inside folder", Source::Agent, None).unwrap();
    let target_folder = root_b.join("Folder");
    events.clear();

    let err = gate::rename(&core, &old_folder, &target_folder).unwrap_err();
    assert!(matches!(&err, crate::files::error::FileCommandError::Task { .. }), "{err:?}");
    assert_eq!(fs::read_to_string(&note).unwrap(), "version inside folder");
    assert!(!target_folder.exists());
    assert_eq!(core.history.page(&|| core.known_roots(), &note, 0, 10).total, 1);
    assert!(!events.names().contains(&"notes-relocated"));
}

#[test]
fn history_cross_vault_movement_cannot_escape_to_unregistered_destination() {
    let (_directory, core, _events, root_a, _root_b) = cross_vault_fixture();
    let from = root_a.join("Outside.md");
    gate::create(&core, &from, "version", Source::Agent, None).unwrap();
    let other = tempfile::tempdir().unwrap();
    let to = other.path().join("Outside.md");

    assert!(gate::move_across(&core, &from, &to).is_err());
    assert!(from.exists());
    assert!(!to.exists());
    assert_eq!(core.history.page(&|| core.known_roots(), &from, 0, 10).total, 1);
}

#[cfg(unix)]
#[test]
fn history_cross_vault_symlink_parent_must_not_escape_to_unregistered_storage() {
    let (_directory, core, events, root_a, _root_b) = cross_vault_fixture();
    let external = tempfile::tempdir().unwrap();
    let external_file = external.path().join("Foreign.md");
    fs::write(&external_file, b"must stay outside").unwrap();
    let alias = root_a.join("alias-outside");
    std::os::unix::fs::symlink(external.path(), &alias).unwrap();

    // Lexically these paths are inside one vault, but one resolves outside.
    // The previous OR-based owner lookup admitted both moves.
    let destination = root_a.join("Imported.md");
    events.clear();
    let error = gate::move_across(&core, &alias.join("Foreign.md"), &destination).unwrap_err();
    assert!(matches!(error, crate::files::error::FileCommandError::Task { .. }));
    assert_eq!(fs::read(&external_file).unwrap(), b"must stay outside");
    assert!(!destination.exists());
    assert!(!events.names().contains(&"notes-relocated"));

    let local = root_a.join("Local.md");
    fs::write(&local, b"must stay inside").unwrap();
    let exported = alias.join("Exported.md");
    let error = gate::move_across(&core, &local, &exported).unwrap_err();
    assert!(matches!(error, crate::files::error::FileCommandError::Task { .. }));
    assert_eq!(fs::read(&local).unwrap(), b"must stay inside");
    assert!(!external.path().join("Exported.md").exists());
}

#[test]
fn history_same_vault_move_still_preserves_versions_with_multiple_roots_registered() {
    let (_directory, core, events, root_a, _root_b) = cross_vault_fixture();
    let from = root_a.join("One.md");
    let to = root_a.join("Two.md");
    gate::create(&core, &from, "still safe", Source::Agent, None).unwrap();
    events.clear();

    gate::move_across(&core, &from, &to).unwrap();
    assert!(!from.exists());
    assert_eq!(fs::read_to_string(&to).unwrap(), "still safe");
    assert_eq!(core.history.page(&|| core.known_roots(), &to, 0, 10).total, 1);
    assert!(events.names().contains(&"notes-relocated"));
}

#[test]
fn core_file_gate_serializes_mcp_trash_purge_across_core_instances() {
    use std::sync::mpsc::{channel, RecvTimeoutError};
    use std::thread;
    use std::time::Duration;

    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().join("vault");
    let trash = workspace.join(".trash");
    fs::create_dir_all(&trash).unwrap();
    let deleted = trash.join("Восстановить.md");
    fs::write(&deleted, b"valuable original").unwrap();

    let data_a = directory.path().join("data-a");
    let data_b = directory.path().join("data-b");
    fs::create_dir_all(&data_a).unwrap();
    fs::create_dir_all(&data_b).unwrap();
    let first = Core::open(&data_a, Arc::new(Recording::default()));
    let second = Core::open(&data_b, Arc::new(Recording::default()));
    for core in [&first, &second] {
        core.ui_state.resolve_workspace(workspace.to_str().unwrap(), 1).unwrap();
    }
    let held = first.file_mutation_guard().unwrap();

    let (started_tx, started_rx) = channel();
    let (finished_tx, finished_rx) = channel();
    let worker = thread::spawn(move || {
        started_tx.send(()).unwrap();
        finished_tx.send(second.empty_trash_guarded(&workspace)).unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        matches!(
            finished_rx.recv_timeout(Duration::from_millis(150)),
            Err(RecvTimeoutError::Timeout)
        ),
        "MCP purge bypassed the process-wide File Gate"
    );
    assert_eq!(fs::read(&deleted).unwrap(), b"valuable original");
    drop(held);

    assert_eq!(finished_rx.recv_timeout(Duration::from_secs(30)).unwrap().unwrap(), 1);
    worker.join().unwrap();
    assert!(!deleted.exists());
}

#[test]
fn the_frontend_receives_the_same_event_names_and_payloads_as_before() {
    let saved = CoreEvent::DocumentSaved(DocumentEvent { path: "a.md".into(), version: 3 });
    assert_eq!(saved.name(), "document-saved");
    assert_eq!(serde_json::to_value(&saved).unwrap(), json!({ "path": "a.md", "version": 3 }));

    let note = CoreEvent::Navigate(Navigation::Note { path: "a.md".into(), disposition: Disposition::NewTab });
    assert_eq!(note.name(), "mcp-navigate");
    assert_eq!(
        serde_json::to_value(&note).unwrap(),
        json!({ "kind": "note", "path": "a.md", "disposition": "new-tab" })
    );
    let workspace = CoreEvent::Navigate(Navigation::Workspace { path: "base".into() });
    assert_eq!(serde_json::to_value(&workspace).unwrap(), json!({ "kind": "workspace", "path": "base" }));

    let changed = CoreEvent::WorkspaceChanged(vec!["a.md".into()]);
    assert_eq!(changed.name(), "workspace-changed");
    assert_eq!(serde_json::to_value(&changed).unwrap(), json!(["a.md"]));
}

#[test]
fn a_replaced_document_announces_the_change_before_the_save() {
    let (directory, core, events) = open_core();
    let note = directory.path().join("Заметка.md");
    fs::write(&note, "первая строка\n").unwrap();
    core.documents.open(&core, &note, 0).unwrap();
    events.clear();

    core.documents
        .replace_text(&core, &note, "первая строка\nвторая\n", SYSTEM_CLIENT, Source::Me, None)
        .unwrap();

    assert_eq!(events.names(), ["document-changed", "document-saved"]);
    assert_eq!(fs::read_to_string(&note).unwrap(), "первая строка\nвторая\n");
}

#[test]
fn a_document_whose_file_vanished_reports_it_missing_instead_of_saved() {
    let (directory, core, events) = open_core();
    let note = directory.path().join("Заметка.md");
    fs::write(&note, "текст\n").unwrap();
    core.documents.open(&core, &note, 0).unwrap();
    fs::remove_file(&note).unwrap();
    events.clear();

    core.documents.replace_text(&core, &note, "новый текст\n", SYSTEM_CLIENT, Source::Me, None).unwrap();

    let names = events.names();
    assert!(names.contains(&"document-missing"), "{names:?}");
    assert!(!names.contains(&"document-saved"), "{names:?}");
}

#[test]
fn trashing_a_note_announces_the_removal() {
    let (directory, core, events) = open_core();
    let workspace = directory.path().join("База");
    fs::create_dir_all(&workspace).unwrap();
    let note = workspace.join("Удалить.md");
    fs::write(&note, "текст\n").unwrap();

    gate::trash(&core, &workspace, &note).unwrap();

    assert!(events.names().contains(&"notes-relocated"), "{:?}", events.names());
    assert!(!Path::new(&note).exists());
}

#[test]
fn a_rename_announces_the_move_before_the_old_path_disappears_from_the_listing() {
    let (directory, core, events) = open_core();
    let workspace = directory.path().join("База");
    fs::create_dir_all(&workspace).unwrap();
    let workspace = fs::canonicalize(workspace).unwrap();
    core.ui_state.resolve_workspace(workspace.to_str().unwrap(), 1).unwrap();
    let note = workspace.join("Без названия.md");
    fs::write(&note, "текст\n").unwrap();
    core.search.prepare(&workspace.to_string_lossy()).unwrap();
    events.clear();

    gate::rename(&core, &note, &workspace.join("Новая.md")).unwrap();

    let names = events.names();
    let moved = names.iter().position(|name| *name == "notes-relocated");
    let listed = names.iter().position(|name| *name == "workspace-changed");
    assert!(moved.is_some() && listed.is_some() && moved < listed, "{names:?}");
}

#[test]
fn watch_batches_reach_the_host_as_the_event_it_can_act_on() {
    use crate::files::watcher::{WatchBatch, WatchScope};
    use std::path::PathBuf;

    let (_directory, core, events) = open_core();
    let batch = |paths: &[&str], scope| WatchBatch::new(paths.iter().map(PathBuf::from).collect(), scope);

    core.ingest_watch(batch(&["a.md"], WatchScope::Content));
    core.ingest_watch(batch(&["b.md"], WatchScope::Structure));
    core.ingest_watch(batch(&["картинка.png"], WatchScope::Listing));
    core.ingest_watch(batch(&["Папка"], WatchScope::Rescan));
    core.ingest_watch(batch(&[], WatchScope::Rescan));

    assert_eq!(events.names(), ["content-changed", "workspace-changed", "workspace-changed", "workspace-changed", "workspace-rescanned"]);
}


#[test]
fn a_reserved_closed_document_cannot_open_in_the_editor_until_released() {
    use crate::documents::hub::{HubError, SyncAdmissionError};
    let dir = tempfile::tempdir().unwrap();
    let app_data = dir.path().join("app-data");
    fs::create_dir_all(&app_data).unwrap();
    let core = Core::open(&app_data, Arc::new(Recording::default()));
    let note = dir.path().join("notes.md");
    fs::write(&note, "unmodified note\n").unwrap();

    let reservation = core.documents.reserve_sync_closed_document(&note).unwrap();
    assert!(matches!(
        core.documents.reserve_sync_closed_document(&note),
        Err(SyncAdmissionError::AlreadyReserved)
    ));
    assert!(matches!(
        core.documents.open(&core, &note, 0),
        Err(HubError::SyncReserved)
    ));
    assert_eq!(fs::read_to_string(&note).unwrap(), "unmodified note\n");

    drop(reservation);
    let opened = core.documents.open(&core, &note, 0).unwrap();
    assert_eq!(opened.text, "unmodified note\n");
    core.documents.release(&core, &note);
}

#[test]
fn even_a_clean_open_editor_session_refuses_sync_reservation() {
    use crate::documents::hub::SyncAdmissionError;
    let dir = tempfile::tempdir().unwrap();
    let app_data = dir.path().join("app-data");
    fs::create_dir_all(&app_data).unwrap();
    let core = Core::open(&app_data, Arc::new(Recording::default()));
    let note = dir.path().join("document.md");
    fs::write(&note, "user text").unwrap();

    core.documents.open(&core, &note, 0).unwrap();
    assert!(matches!(
        core.documents.reserve_sync_closed_document(&note),
        Err(SyncAdmissionError::DocumentOpen)
    ));
    core.documents.release(&core, &note);
    let admission = core.documents.reserve_sync_closed_document(&note).unwrap();
    assert_eq!(fs::read_to_string(&note).unwrap(), "user text");
    drop(admission);
}

#[test]
fn sync_reservation_only_blocks_its_own_document_identity() {
    let dir = tempfile::tempdir().unwrap();
    let app_data = dir.path().join("app-data");
    fs::create_dir_all(&app_data).unwrap();
    let core = Core::open(&app_data, Arc::new(Recording::default()));
    let a = dir.path().join("a.md");
    let b = dir.path().join("b.md");
    fs::write(&a, "alpha").unwrap();
    fs::write(&b, "beta").unwrap();

    let _reservation = core.documents.reserve_sync_closed_document(&a).unwrap();
    let opened = core.documents.open(&core, &b, 0).unwrap();
    assert_eq!(opened.text, "beta");
    core.documents.release(&core, &b);
    assert_eq!(fs::read_to_string(&a).unwrap(), "alpha");
}


#[test]
fn sync_reservation_denies_an_editor_open_from_another_thread() {
    use crate::documents::hub::HubError;
    let dir = tempfile::tempdir().unwrap();
    let app_data = dir.path().join("app-data");
    fs::create_dir_all(&app_data).unwrap();
    let core = Core::open(&app_data, Arc::new(Recording::default()));
    let note = dir.path().join("threaded.md");
    fs::write(&note, "original contents").unwrap();

    let reservation = core.documents.reserve_sync_closed_document(&note).unwrap();
    let other_core = Arc::clone(&core);
    let other_note = note.clone();
    let denied = std::thread::spawn(move || {
        matches!(
            other_core.documents.open(&other_core, &other_note, 0),
            Err(HubError::SyncReserved)
        )
    }).join().unwrap();
    assert!(denied, "editor open crossed an active sync reservation");
    assert_eq!(fs::read_to_string(&note).unwrap(), "original contents");

    drop(reservation);
    let opened = core.documents.open(&core, &note, 0).unwrap();
    assert_eq!(opened.text, "original contents");
    core.documents.release(&core, &note);
}

#[test]
fn core_watcher_forwards_txt_create_modify_rename_and_delete_to_search() {
    use crate::files::watcher::{WatchBatch, WatchScope};
    use crate::search::models::SearchIndexState;
    use crate::search::query::Request;
    use std::time::{Duration, Instant};

    fn wait(stage: &str, mut predicate: impl FnMut() -> bool) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(25) {
            if predicate() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("{stage}: search did not reach the expected state");
    }

    fn hit(core: &Core, workspace: &str, term: &str, expected: &Path) -> bool {
        core.search
            .search(workspace, &Request::new(term))
            .unwrap()
            .results
            .iter()
            .any(|result| crate::search::paths::same_path(Path::new(&result.path), expected))
    }

    let (directory, core, _events) = open_core();
    let root = directory.path().join("Vault");
    fs::create_dir_all(&root).unwrap();
    let root = fs::canonicalize(&root).unwrap();
    let workspace = root.to_string_lossy().into_owned();
    core.ui_state.resolve_workspace(&workspace, 1).unwrap();
    let md = root.join("Control.md");
    let baseline = b"desk10baselinecontrol only\n";
    fs::write(&md, baseline).unwrap();

    core.search.prepare(&workspace).unwrap();
    wait("initial index", || {
        let status = core.search.status(Some(&workspace));
        status.state == SearchIndexState::Ready && !status.updating
    });
    assert!(hit(&core, &workspace, "desk10baselinecontrol", &md));

    // This calls Core::ingest_watch, the exact production bridge from the
    // notify watcher callback to SearchService (not a test-only sink).
    let txt = root.join("東京日記.TXT");
    let first = "日本語と中文\ndesk10corecreatedneedle\n";
    fs::write(&txt, first).unwrap();
    core.ingest_watch(WatchBatch::new(vec![txt.clone()], WatchScope::Structure));
    wait("new TXT event", || {
        hit(&core, &workspace, "desk10corecreatedneedle", &txt)
    });
    assert_eq!(fs::read_to_string(&txt).unwrap(), first);

    let updated = "日本語と中文と한국어\ndesk10corechangedneedle\nsecond line\n";
    fs::write(&txt, updated).unwrap();
    core.ingest_watch(WatchBatch::new(vec![txt.clone()], WatchScope::Content));
    wait("modified TXT event", || {
        hit(&core, &workspace, "desk10corechangedneedle", &txt)
            && !hit(&core, &workspace, "desk10corecreatedneedle", &txt)
    });
    assert_eq!(fs::read_to_string(&txt).unwrap(), updated);

    let moved = root.join("東京日記改.txt");
    fs::rename(&txt, &moved).unwrap();
    core.ingest_watch(WatchBatch::new(
        vec![txt.clone(), moved.clone()],
        WatchScope::Structure,
    ));
    wait("renamed TXT event", || {
        hit(&core, &workspace, "desk10corechangedneedle", &moved)
            && !hit(&core, &workspace, "desk10corechangedneedle", &txt)
    });
    assert_eq!(fs::read_to_string(&moved).unwrap(), updated);

    fs::remove_file(&moved).unwrap();
    core.ingest_watch(WatchBatch::new(vec![moved.clone()], WatchScope::Structure));
    wait("deleted TXT event", || {
        !hit(&core, &workspace, "desk10corechangedneedle", &moved)
    });
    assert!(hit(&core, &workspace, "desk10baselinecontrol", &md));
    assert_eq!(fs::read(&md).unwrap(), baseline);
    println!(
        "DESK10_CORE_TXT_WATCH_FORWARD_PASS create=1 modify=1 rename=1 delete=1 md_preserved=1"
    );

    let opened = core.search.active.write().unwrap().take();
    if let Some(index) = opened {
        index.stop();
    }
}
