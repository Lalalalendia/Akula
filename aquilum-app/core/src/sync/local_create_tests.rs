//! SYNC-04g-M1: additive-only local creation negative tests.
//! These tests do not enable the production sync executor.

use super::*;
use crate::app_core::{Core, CoreEvent};
use crate::sync::{DocumentId, OperationIntent, RevisionId, VaultId};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn core_at(root: &Path, appdata: &Path) -> Arc<Core> {
    fs::create_dir_all(appdata).unwrap();
    let core = Core::open(appdata, Arc::new(|_: CoreEvent| {}));
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    core
}

fn fixture() -> (tempfile::TempDir, Arc<Core>, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    fs::create_dir(&root).unwrap();
    let core = core_at(&root, &temp.path().join("appdata"));
    (temp, core, root)
}

fn staged(
    core: &Core, db_dir: &Path, relative: &str, bytes: &[u8], operation: OperationId,
) -> (SqliteSyncJournal, SqliteDagStore, VaultId, DocumentId) {
    fs::create_dir_all(db_dir).unwrap();
    let vault = VaultId::from_uuid(core.ui_state.list_workspaces().unwrap()[0].id);
    let doc = DocumentId::new();
    let intent = OperationIntent {
        operation_id: operation, vault_id: vault, document_id: doc,
        profile_id: "m1-fixture".into(), revision_id: RevisionId::new(),
        action: JournalAction::StageDownload,
        path: SyncRelativePath::parse(relative).unwrap(),
        remote_object_id: format!("immutable/{operation}"),
        remote_guard: None,
        expected_content_hash: blake3::hash(bytes).to_hex().to_string(),
        expected_baseline: None,
    };
    let mut journal = SqliteSyncJournal::open(&db_dir.join("ops.sqlite")).unwrap();
    let mut cache = SqliteDagStore::open(&db_dir.join("blobs.sqlite")).unwrap();
    assert_eq!(cache.store_blob(bytes).unwrap(), intent.expected_content_hash);
    assert_eq!(journal.begin(intent).unwrap(), JournalPhase::Pending);
    assert_eq!(journal.mark_staged(operation, bytes).unwrap(), JournalPhase::Staged);
    (journal, cache, vault, doc)
}

#[test]
fn m1_create_is_complete_no_replace_and_leaves_baseline_unconfirmed() {
    let (temp, core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let relative = "notes/Глава 140 😀.md";
    let bytes = "Глава 140: новая заметка 😀\n".as_bytes();
    let id = OperationId::new();
    let (journal, cache, vault, doc) = staged(
        &core, &temp.path().join("appdata"), relative, bytes, id,
    );
    let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
    let result = admission.create_staged_if_absent(id, &journal, &cache).unwrap();
    assert_eq!(result.byte_length, bytes.len() as u64);
    assert_eq!(fs::read(root.join(relative)).unwrap(), bytes);
    assert!(!root.join("notes").join(format!(".akula-sync-stage-{id}")).exists());
    assert!(matches!(
        admission.create_staged_if_absent(id, &journal, &cache),
        Err(LocalCreateError::Admission(VaultPortError::DestinationExists))
    ));
    assert_eq!(fs::read(root.join(relative)).unwrap(), bytes);
    assert_eq!(journal.get(id).unwrap().unwrap().phase, JournalPhase::Staged);
    assert!(journal.baseline(vault, doc).unwrap().is_none());
}

#[test]
fn m1_rejects_unicode_case_collisions_and_document_reservation() {
    let (temp, core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let conflicting = root.join("notes/cafe\u{301}.md");
    fs::write(&conflicting, b"keep original").unwrap();
    let id = OperationId::new();
    let (journal, cache, vault, doc) = staged(
        &core, &temp.path().join("appdata"), "notes/Café.md", b"new", id,
    );
    let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
    assert!(matches!(
        admission.create_staged_if_absent(id, &journal, &cache),
        Err(LocalCreateError::Admission(VaultPortError::DestinationExists))
    ));
    assert_eq!(fs::read(&conflicting).unwrap(), b"keep original");
    fs::remove_file(conflicting).unwrap();
    let target = root.join("notes/Café.md");
    let held = core.documents.reserve_sync_closed_document(&target).unwrap();
    assert!(admission.create_staged_if_absent(id, &journal, &cache).is_err());
    drop(held);
    assert!(!target.exists());
    assert!(journal.baseline(vault, doc).unwrap().is_none());
}

#[test]
fn m1_refuses_missing_intent_and_wrong_registered_vault() {
    let (temp, core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let id = OperationId::new();
    let (journal, cache, _, _) = staged(
        &core, &temp.path().join("appdata"), "notes/fresh.md", b"new", id,
    );
    let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
    assert!(matches!(
        admission.create_staged_if_absent(OperationId::new(), &journal, &cache),
        Err(LocalCreateError::InvalidStagedIntent)
    ));
    let other = tempfile::tempdir().unwrap();
    let other_root = other.path().join("vault");
    fs::create_dir(&other_root).unwrap();
    let other_core = core_at(&other_root, &other.path().join("appdata"));
    let foreign_id = OperationId::new();
    let (foreign_journal, foreign_cache, _, _) = staged(
        &other_core, &other.path().join("appdata"),
        "notes/other.md", b"outsider", foreign_id,
    );
    assert!(matches!(
        admission.create_staged_if_absent(foreign_id, &foreign_journal, &foreign_cache),
        Err(LocalCreateError::InvalidStagedIntent)
    ));
    assert!(!root.join("notes/other.md").exists());
}

#[cfg(unix)]
#[test]
fn m1_rejects_symlinked_intermediate_directory_without_escape() {
    use std::os::unix::fs::symlink;
    let (temp, core, root) = fixture();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, root.join("notes")).unwrap();
    let id = OperationId::new();
    let (journal, cache, _, _) = staged(
        &core, &temp.path().join("appdata"), "notes/outside.md", b"unsafe", id,
    );
    assert!(RegisteredReadAdmission::open(&core, &root).unwrap()
        .create_staged_if_absent(id, &journal, &cache).is_err());
    assert!(!outside.join("outside.md").exists());
}

#[cfg(windows)]
#[test]
fn m1_rejects_preexisting_ntfs_junction_parent() {
    use std::process::Command;
    let (temp, core, root) = fixture();
    let outside = temp.path().join("external");
    fs::create_dir(&outside).unwrap();
    let parent = root.join("notes");
    let status = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&parent)
        .arg(&outside)
        .status().unwrap();
    assert!(status.success(), "junction fixture must exist for this security test");
    let id = OperationId::new();
    let (journal, cache, vault, doc) = staged(
        &core, &temp.path().join("appdata"), "notes/remote.md", b"new", id,
    );
    let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
    assert!(admission.create_staged_if_absent(id, &journal, &cache).is_err());
    assert!(!outside.join("remote.md").exists());
    assert!(journal.baseline(vault, doc).unwrap().is_none());
}

#[cfg(unix)]
#[test]
fn m1_parent_swapped_to_symlink_before_publish_never_writes_outside() {
    use std::os::unix::fs::symlink;
    let (temp, core, root) = fixture();
    let original = root.join("notes");
    fs::create_dir(&original).unwrap();
    let outside = temp.path().join("external");
    fs::create_dir(&outside).unwrap();
    let id = OperationId::new();
    let bytes = b"safe staged";
    let (journal, cache, vault, doc) = staged(
        &core, &temp.path().join("appdata"), "notes/remote.md", bytes, id,
    );
    let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
    let moved = temp.path().join("moved-parent");
    let result = admission.create_staged_with_hook(id, &journal, &cache, || {
        fs::rename(&original, &moved).unwrap();
        symlink(&outside, &original).unwrap();
    });
    assert!(matches!(result, Err(LocalCreateError::PublicationUncertain)));
    // Publication was relative to the ORIGINAL opened directory, never the
    // path later redirected to an external directory by another process.
    assert_eq!(fs::read(moved.join("remote.md")).unwrap(), bytes);
    assert!(!outside.join("remote.md").exists());
    assert!(journal.baseline(vault, doc).unwrap().is_none());
}

/// Runs ONLY in a child test process; accepts the same source-level contracts
/// but authenticates a separate Core and SQLite journal.
#[test]
fn m1_external_process_fixture() {
    let Some(root) = std::env::var_os("AKULA_M1_CHILD_ROOT") else { return };
    let root = PathBuf::from(root);
    let appdata = PathBuf::from(std::env::var_os("AKULA_M1_CHILD_APPDATA").unwrap());
    let relative = std::env::var("AKULA_M1_CHILD_RELATIVE").unwrap();
    let operation = std::env::var("AKULA_M1_CHILD_OP").unwrap().parse().unwrap();
    let core = core_at(&root, &appdata);
    let bytes: &[u8] = match std::env::var("AKULA_M1_CHILD_ACTION").unwrap().as_str() {
        "compete" => b"child won atomically",
        "crash" => b"staged before process exit",
        _ => panic!("unknown child action"),
    };
    let (journal, cache, _, _) = staged(&core, &appdata, &relative, bytes, operation);
    let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
    if std::env::var("AKULA_M1_CHILD_ACTION").unwrap() == "crash" {
        admission.create_staged_with_hook(operation, &journal, &cache, || {
            std::process::exit(87);
        }).unwrap();
        unreachable!("child must terminate before publishing");
    }
    admission.create_staged_if_absent(operation, &journal, &cache).unwrap();
    let marker = std::env::var_os("AKULA_M1_CHILD_MARKER").unwrap();
    fs::write(marker, b"child completed").unwrap();
}

fn run_child(
    temp: &tempfile::TempDir, root: &Path, relative: &str,
    operation: OperationId, action: &str,
) -> std::process::Output {
    use std::process::Command;
    Command::new(std::env::current_exe().unwrap())
        .arg("m1_external_process_fixture")
        .arg("--nocapture")
        .env("AKULA_M1_CHILD_ROOT", root)
        .env("AKULA_M1_CHILD_APPDATA", temp.path().join("child-appdata"))
        .env("AKULA_M1_CHILD_RELATIVE", relative)
        .env("AKULA_M1_CHILD_OP", operation.to_string())
        .env("AKULA_M1_CHILD_ACTION", action)
        .env("AKULA_M1_CHILD_MARKER", temp.path().join("child-marker"))
        .output().unwrap()
}

#[test]
fn m1_foreign_process_wins_publish_race_without_overwrite() {
    let (temp, core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let relative = "notes/contested.md";
    let id = OperationId::new();
    let (journal, cache, vault, doc) = staged(
        &core, &temp.path().join("appdata"), relative, b"parent original", id,
    );
    let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
    let rival = OperationId::new();
    let result = admission.create_staged_with_hook(id, &journal, &cache, || {
        let output = run_child(&temp, &root, relative, rival, "compete");
        assert!(output.status.success(), "child: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(fs::read(temp.path().join("child-marker")).unwrap(), b"child completed");
    });
    assert!(matches!(result,
        Err(LocalCreateError::Admission(VaultPortError::DestinationExists))));
    assert_eq!(fs::read(root.join(relative)).unwrap(), b"child won atomically");
    assert_eq!(fs::read(root.join("notes").join(format!(".akula-sync-stage-{id}"))).unwrap(),
        b"parent original");
    assert!(journal.baseline(vault, doc).unwrap().is_none());
}

#[test]
fn m1_real_process_exit_after_staging_recovers_without_false_ack() {
    let (temp, _core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let id = OperationId::new();
    let relative = "notes/crash.md";
    let output = run_child(&temp, &root, relative, id, "crash");
    assert_eq!(output.status.code(), Some(87),
        "child did not reach the stage-sync crash point: {}",
        String::from_utf8_lossy(&output.stderr));
    assert!(!root.join(relative).exists());
    assert_eq!(
        fs::read(root.join("notes").join(format!(".akula-sync-stage-{id}"))).unwrap(),
        b"staged before process exit"
    );
    let db = temp.path().join("child-appdata");
    let journal = SqliteSyncJournal::open(&db.join("ops.sqlite")).unwrap();
    let record = journal.get(id).unwrap().unwrap();
    assert_eq!(record.phase, JournalPhase::Staged);
    assert!(journal.baseline(record.intent.vault_id, record.intent.document_id)
        .unwrap().is_none());
}
