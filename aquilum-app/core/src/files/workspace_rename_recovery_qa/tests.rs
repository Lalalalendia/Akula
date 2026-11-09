use super::*;
use std::fs;

fn fixture() -> (tempfile::TempDir, RenameIntent, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("Before");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("note.md"), "original").unwrap();
    let dest = temp.path().join("After");
    let root = physical_root(&source).unwrap().unwrap();
    let intent = RenameIntent {
        schema: SCHEMA,
        operation_id: Uuid::new_v4(),
        workspace_id: Uuid::new_v4(),
        source,
        destination: dest,
        expected_root: root,
    };
    let journal = temp.path().join("operation-v1.json");
    (temp, intent, journal)
}

#[test]
fn pre_move_journal_is_readable_and_does_not_move_any_data() {
    let (_temp, intent, journal) = fixture();
    write_new_journal(&journal, &intent).unwrap();
    let stored = read_journal(&journal).unwrap();
    assert_eq!(stored.operation_id, intent.operation_id);
    assert_eq!(stored.workspace_id, intent.workspace_id);
    assert_eq!(stored.expected_root, intent.expected_root);
    assert_eq!(classify_journal(&journal, Some((intent.workspace_id, &intent.source))), RecoveryView::Prepared);
    assert_eq!(fs::read_to_string(intent.source.join("note.md")).unwrap(), "original");
    assert!(!intent.destination.exists());
}

#[test]
fn move_before_sqlite_cas_is_detected_but_not_replayed() {
    let (_temp, intent, journal) = fixture();
    write_new_journal(&journal, &intent).unwrap();
    fs::rename(&intent.source, &intent.destination).unwrap(); // temp fixture only
    assert_eq!(classify_journal(&journal, Some((intent.workspace_id, &intent.source))),
        RecoveryView::MovedWaitingForRegistry);
    assert_eq!(fs::read_to_string(intent.destination.join("note.md")).unwrap(), "original");
    assert!(!intent.source.exists());
}

#[test]
fn cas_complete_before_ack_is_detected_but_not_auto_cleaned() {
    let (_temp, intent, journal) = fixture();
    write_new_journal(&journal, &intent).unwrap();
    fs::rename(&intent.source, &intent.destination).unwrap(); // temp fixture only
    assert_eq!(classify_journal(&journal, Some((intent.workspace_id, &intent.destination))),
        RecoveryView::RegistryChangedWaitingForAck);
    assert!(journal.is_file(), "journal is intentionally not removed or acknowledged");
}

#[test]
fn duplicate_journal_create_new_preserves_original() {
    let (_temp, intent, journal) = fixture();
    write_new_journal(&journal, &intent).unwrap();
    let original = fs::read(&journal).unwrap();
    assert_eq!(write_new_journal(&journal, &intent).unwrap_err().kind(), ErrorKind::AlreadyExists);
    assert_eq!(fs::read(journal).unwrap(), original);
}

#[test]
fn truncated_corrupt_and_unknown_schema_hold() {
    let (_temp, mut intent, journal) = fixture();
    fs::write(&journal, b"{").unwrap();
    assert_eq!(classify_journal(&journal, Some((intent.workspace_id, &intent.source))), RecoveryView::Hold);
    intent.schema += 1;
    let altered = serde_json::to_vec(&intent).unwrap();
    fs::write(&journal, altered).unwrap();
    assert_eq!(classify_journal(&journal, Some((intent.workspace_id, &intent.source))), RecoveryView::Hold);
}

#[test]
fn mismatched_uuid_and_registry_path_hold() {
    let (_temp, intent, journal) = fixture();
    write_new_journal(&journal, &intent).unwrap();
    assert_eq!(classify_journal(&journal, Some((Uuid::new_v4(), &intent.source))), RecoveryView::Hold);
    assert_eq!(classify_journal(&journal, None), RecoveryView::Hold);
    assert_eq!(classify_journal(&journal, Some((intent.workspace_id, &intent.destination))), RecoveryView::Hold);
}

#[test]
fn both_old_and_new_existing_hold_without_modification() {
    let (_temp, intent, journal) = fixture();
    write_new_journal(&journal, &intent).unwrap();
    fs::create_dir(&intent.destination).unwrap();
    fs::write(intent.destination.join("note.md"), "different data").unwrap();
    assert_eq!(classify_journal(&journal, Some((intent.workspace_id, &intent.source))), RecoveryView::Hold);
    assert_eq!(fs::read_to_string(intent.source.join("note.md")).unwrap(), "original");
    assert_eq!(fs::read_to_string(intent.destination.join("note.md")).unwrap(), "different data");
}

#[test]
fn neither_root_existing_holds() {
    let (_temp, intent, journal) = fixture();
    write_new_journal(&journal, &intent).unwrap();
    fs::rename(&intent.source, intent.source.with_file_name("Unknown")).unwrap(); // fixture only
    assert_eq!(classify_journal(&journal, Some((intent.workspace_id, &intent.source))), RecoveryView::Hold);
}

#[test]
fn destination_with_wrong_physical_identity_holds() {
    let (_temp, intent, journal) = fixture();
    write_new_journal(&journal, &intent).unwrap();
    fs::rename(&intent.source, intent.source.with_file_name("Unknown")).unwrap();
    fs::create_dir(&intent.destination).unwrap();
    assert_eq!(classify_journal(&journal, Some((intent.workspace_id, &intent.source))), RecoveryView::Hold);
}

#[test]
fn non_sibling_and_traversal_intents_are_rejected() {
    let (_temp, mut intent, journal) = fixture();
    intent.destination = intent.source.join("Nested");
    assert_eq!(write_new_journal(&journal, &intent).unwrap_err().kind(), ErrorKind::InvalidData);
    intent.destination = intent.source.with_file_name("../Alias");
    assert_eq!(write_new_journal(&journal, &intent).unwrap_err().kind(), ErrorKind::InvalidData);
    assert!(!journal.exists());
}

#[cfg(unix)]
#[test]
fn destination_symlink_is_a_hold_not_followed() {
    use std::os::unix::fs::symlink;
    let (temp, intent, journal) = fixture();
    write_new_journal(&journal, &intent).unwrap();
    let outside = temp.path().join("Outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("private.md"), "keep").unwrap();
    symlink(&outside, &intent.destination).unwrap();
    assert_eq!(classify_journal(&journal, Some((intent.workspace_id, &intent.source))), RecoveryView::Hold);
    assert_eq!(fs::read_to_string(outside.join("private.md")).unwrap(), "keep");
}

#[cfg(windows)]
#[test]
fn destination_junction_is_a_hold_not_followed() {
    use std::process::Command;
    let (temp, intent, journal) = fixture();
    write_new_journal(&journal, &intent).unwrap();
    let outside = temp.path().join("Outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("private.md"), "keep").unwrap();
    let out = Command::new("cmd").arg("/C").arg("mklink").arg("/J")
        .arg(&intent.destination).arg(&outside).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(classify_journal(&journal, Some((intent.workspace_id, &intent.source))), RecoveryView::Hold);
    assert_eq!(fs::read_to_string(outside.join("private.md")).unwrap(), "keep");
}

#[test]
fn journal_inside_vault_is_rejected() {
    let (_temp, intent, _journal) = fixture();
    let bad = intent.source.join("intent.json");
    assert_eq!(write_new_journal(&bad, &intent).unwrap_err().kind(), ErrorKind::InvalidData);
    assert!(!bad.exists());
}
