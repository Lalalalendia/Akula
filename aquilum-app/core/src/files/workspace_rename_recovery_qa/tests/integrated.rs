//! Test-only integration of real SQLite CAS, read-only preflight, native
//! no-replace directory move and the external crash-intent classifier.
use super::*;
use crate::files::workspace_no_replace_qa::no_replace_directory_rename;
use crate::ui_state::UiStateService;
use std::path::PathBuf;

fn with_registry() -> (tempfile::TempDir, UiStateService, RenameIntent, PathBuf) {
    let (temp, mut intent, journal) = fixture();
    // macOS temporary directories often start with /var -> /private/var.
    // Register the real path, not a symlinked ancestor; production preflight
    // must keep refusing aliases rather than allowing them for a test.
    intent.source = fs::canonicalize(&intent.source).unwrap();
    intent.destination = intent.source.with_file_name("After");
    let service = UiStateService::open(&temp.path().join("ui-state.sqlite"));
    let uuid = service.resolve_workspace(&intent.source.to_string_lossy(), 1).unwrap();
    let known = service.list_workspaces().unwrap();
    let current = known.iter().find(|w| w.id == uuid).unwrap();
    intent.workspace_id = uuid;
    // Use Akula's registered spelling, including macOS /private/var and
    // Windows NT path normalization, for both CAS and intent fields.
    intent.source = PathBuf::from(&current.path);
    intent.destination = intent.source.with_file_name("After");
    intent.expected_root = physical_root(&intent.source).unwrap().unwrap();
    (temp, service, intent, journal)
}

fn stored_path(service: &UiStateService, id: Uuid) -> PathBuf {
    let known = service.list_workspaces().unwrap();
    PathBuf::from(&known.iter().find(|w| w.id == id).unwrap().path)
}

fn full_registry(service: &UiStateService) -> Vec<(Uuid, PathBuf)> {
    service.list_workspaces().unwrap().into_iter()
        .map(|known| (known.id, PathBuf::from(known.path))).collect()
}

#[test]
fn state_machine_follows_real_filesystem_move_and_sqlite_cas_without_automatic_recovery() {
    let (_tmp, service, intent, journal) = with_registry();
    // Whole-vault rename must preserve history bytes physically, unlike
    // a content-only duplicate that intentionally omits service metadata.
    let history = intent.source.join(".aquilum/history");
    fs::create_dir_all(&history).unwrap();
    fs::write(history.join("version.bin"), [0_u8, 255, 12]).unwrap();
    let planned = service.plan_inactive_workspace_rename(intent.workspace_id, "After", None).unwrap();
    assert_eq!(planned.destination_path, intent.destination);
    write_new_journal(&journal, &intent).unwrap();
    let old = stored_path(&service, intent.workspace_id);
    assert_eq!(classify_journal_with_registry(&journal, &full_registry(&service)),
        RecoveryView::Prepared);

    no_replace_directory_rename(&intent.source, &intent.destination).unwrap();
    assert_eq!(stored_path(&service, intent.workspace_id), old);
    assert_eq!(classify_journal_with_registry(&journal, &full_registry(&service)),
        RecoveryView::MovedWaitingForRegistry);

    service.replace_workspace_path_after_move(
        intent.workspace_id, &old.to_string_lossy(), &intent.destination.to_string_lossy(),
    ).unwrap();
    let new = stored_path(&service, intent.workspace_id);
    assert_eq!(new, intent.destination);
    assert_eq!(classify_journal_with_registry(&journal, &full_registry(&service)),
        RecoveryView::RegistryChangedWaitingForAck);
    assert!(journal.is_file(), "no journal cleanup or ACK without host custody");
    assert_eq!(fs::read_to_string(intent.destination.join("note.md")).unwrap(), "original");
    assert_eq!(fs::read(intent.destination.join(".aquilum/history/version.bin")).unwrap(), [0, 255, 12]);
}

#[test]
fn registry_race_after_preflight_does_not_erase_moved_data_or_reassign_uuid() {
    let (_tmp, service, intent, journal) = with_registry();
    service.plan_inactive_workspace_rename(intent.workspace_id, "After", None).unwrap();
    write_new_journal(&journal, &intent).unwrap();
    // Simulate an unrelated registry writer between preflight and move.
    let other = service.resolve_workspace(&intent.destination.to_string_lossy(), 2).unwrap();
    assert_ne!(other, intent.workspace_id);
    no_replace_directory_rename(&intent.source, &intent.destination).unwrap();
    let old = stored_path(&service, intent.workspace_id);
    assert!(service.replace_workspace_path_after_move(
        intent.workspace_id, &old.to_string_lossy(), &intent.destination.to_string_lossy(),
    ).is_err());
    // A competing UUID at the moved destination is ambiguous. Do not
    // classify it as CAS-pending just because this one UUID still points old.
    assert_eq!(classify_journal_with_registry(&journal, &full_registry(&service)),
        RecoveryView::Hold);
    assert_eq!(stored_path(&service, other), intent.destination);
    assert!(!intent.source.exists());
    assert_eq!(fs::read_to_string(intent.destination.join("note.md")).unwrap(), "original");
    assert!(journal.is_file());
}

mod kill;
