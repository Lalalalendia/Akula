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
    #[cfg(unix)]
    {
        let observation = journal.read_native_local_publication_observation(id)
            .unwrap().expect("Unix parent fsync and pinned readback must be audited");
        assert_eq!(observation.operation_id, id);
        assert_eq!(observation.vault_id, vault);
        assert_eq!(observation.final_file, result.file);
        assert_eq!(observation.byte_length, bytes.len() as u64);
        assert_eq!(observation.expected_content_hash,
            blake3::hash(bytes).to_hex().to_string());
        assert_eq!(observation.path.as_str(), relative);
    }
    #[cfg(not(unix))]
    assert!(journal.read_native_local_publication_observation(id).unwrap().is_none());

    assert!(!root.join("notes").join(format!(".akula-sync-stage-{id}")).exists());
    assert!(matches!(
        admission.create_staged_if_absent(id, &journal, &cache),
        Err(LocalCreateError::Admission(VaultPortError::DestinationExists))
    ));
    assert_eq!(fs::read(root.join(relative)).unwrap(), bytes);
    assert_eq!(journal.get(id).unwrap().unwrap().phase, JournalPhase::Staged);
    assert!(journal.baseline(vault, doc).unwrap().is_none());
    assert_eq!(journal.local_publication_review_queue().unwrap().len(), 1);
    // Even if an external user deletes the published file, replaying the
    // same operation is prohibited by its durable SQLite admission record.
    fs::remove_file(root.join(relative)).unwrap();
    assert!(matches!(
        admission.create_staged_if_absent(id, &journal, &cache),
        Err(LocalCreateError::Journal(
            JournalError::PublicationAttemptAlreadyRecorded))
    ));
    assert!(!root.join(relative).exists());
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
    let action = std::env::var("AKULA_M1_CHILD_ACTION").unwrap();
    if action == "compete" {
        // This foreign process intentionally ignores Akula's in-process
        // File Gate and journal; it atomically creates the competing name.
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true)
            .open(root.join(relative)).unwrap();
        file.write_all(b"child won atomically").unwrap();
        file.sync_all().unwrap();
        let marker = std::env::var_os("AKULA_M1_CHILD_MARKER").unwrap();
        fs::write(marker, b"child completed").unwrap();
        return;
    }
    assert!(action == "crash" || action == "published");
    let core = core_at(&root, &appdata);
    let bytes: &[u8] = if action == "crash" {
        b"staged before process exit"
    } else {
        b"published before lost acknowledgement"
    };
    let (journal, cache, _, _) = staged(&core, &appdata, &relative, bytes, operation);
    let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
    if action == "crash" {
        admission.create_staged_with_hook(operation, &journal, &cache, || {
            std::process::exit(87);
        }).unwrap();
        unreachable!("child must terminate before publishing");
    }
    admission.create_staged_if_absent(operation, &journal, &cache).unwrap();
    std::process::exit(88);
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
    // The process stopped before no-replace, pinned final readback and
    // directory fsync: no native publication observation may be synthesized.
    assert!(journal.read_native_local_publication_observation(id).unwrap().is_none());
    assert!(journal.baseline(record.intent.vault_id, record.intent.document_id)
        .unwrap().is_none());
    let pending = journal.local_publication_review_queue().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].operation_id, id);
    // A distinct Core instance after restart cannot accidentally replay,
    // even when a user manually moved away the orphan stage file.
    let cache = SqliteDagStore::open(&db.join("blobs.sqlite")).unwrap();
    let stage_path = root.join("notes").join(format!(".akula-sync-stage-{id}"));
    fs::remove_file(stage_path).unwrap();
    let reopened_core = core_at(&root, &db);
    let admission = RegisteredReadAdmission::open(&reopened_core, &root).unwrap();
    assert!(matches!(
        admission.create_staged_if_absent(id, &journal, &cache),
        Err(LocalCreateError::Journal(
            JournalError::PublicationAttemptAlreadyRecorded))
    ));
    assert!(!root.join(relative).exists());
}


fn m1c_claim(journal: &SqliteSyncJournal, operation: OperationId) {
    let intent = journal.get(operation).unwrap().unwrap().intent;
    journal.record_local_publication_attempt_once(&intent).unwrap();
}

#[test]
fn m1c_claim_without_any_files_is_an_unconfirmed_empty_observation() {
    let (temp, core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let op = OperationId::new();
    let (journal, _cache, vault, doc) = staged(
        &core, &temp.path().join("appdata"), "notes/recovery.md", b"expected", op,
    );
    m1c_claim(&journal, op);
    let host = RegisteredReadAdmission::open(&core, &root).unwrap();
    let evidence = host.inspect_local_publication(op, &journal).unwrap();
    assert_eq!(evidence.assessment,
        local_recovery::LocalRecoveryAssessment::NoVisibleNames);
    assert_eq!(evidence.stage, local_recovery::ObservedLocalFile::Absent);
    assert_eq!(evidence.final_name, local_recovery::ObservedLocalFile::Absent);
    assert!(journal.baseline(vault, doc).unwrap().is_none());
    assert_eq!(journal.local_publication_review_queue().unwrap().len(), 1);
}

#[test]
fn m1c_real_process_crash_stage_is_identified_without_mutating_it() {
    let (temp, _core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let op = OperationId::new();
    let relative = "notes/crash-m1c.md";
    let output = run_child(&temp, &root, relative, op, "crash");
    assert_eq!(output.status.code(), Some(87),
        "crash fixture did not run: {}", String::from_utf8_lossy(&output.stderr));
    let appdata = temp.path().join("child-appdata");
    let core = core_at(&root, &appdata);
    let journal = SqliteSyncJournal::open(&appdata.join("ops.sqlite")).unwrap();
    let host = RegisteredReadAdmission::open(&core, &root).unwrap();
    let evidence = host.inspect_local_publication(op, &journal).unwrap();
    assert_eq!(evidence.assessment,
        local_recovery::LocalRecoveryAssessment::StagedOnly);
    assert!(matches!(
        evidence.stage,
        local_recovery::ObservedLocalFile::Present {
            matches_expected_hash: true, .. }
    ));
    assert_eq!(evidence.final_name, local_recovery::ObservedLocalFile::Absent);
    assert_eq!(fs::read(root.join("notes").join(format!(
        ".akula-sync-stage-{op}"
    ))).unwrap(), b"staged before process exit");
    assert!(!root.join(relative).exists());
    assert_eq!(journal.local_publication_review_queue().unwrap().len(), 1);
    let record = journal.get(op).unwrap().unwrap();
    assert!(journal.baseline(record.intent.vault_id, record.intent.document_id)
        .unwrap().is_none());
}

#[test]
fn m1c_completed_local_name_is_matching_evidence_not_a_baseline_receipt() {
    let (temp, core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let op = OperationId::new();
    let relative = "notes/new.md";
    let contents = b"matching data";
    let (journal, cache, vault, doc) = staged(
        &core, &temp.path().join("appdata"), relative, contents, op,
    );
    let host = RegisteredReadAdmission::open(&core, &root).unwrap();
    host.create_staged_if_absent(op, &journal, &cache).unwrap();
    let evidence = host.inspect_local_publication(op, &journal).unwrap();
    assert_eq!(evidence.assessment,
        local_recovery::LocalRecoveryAssessment::FinalNameMatchesExpected);
    assert_eq!(evidence.stage, local_recovery::ObservedLocalFile::Absent);
    assert!(matches!(evidence.final_name,
        local_recovery::ObservedLocalFile::Present {
            matches_expected_hash: true, byte_length: 13, .. }));
    assert_eq!(fs::read(root.join(relative)).unwrap(), contents);
    assert_eq!(journal.get(op).unwrap().unwrap().phase, JournalPhase::Staged);
    assert!(journal.baseline(vault, doc).unwrap().is_none());
}

#[test]
fn m1c_foreign_file_and_corrupted_stage_are_untrusted_without_deletion() {
    let (temp, core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let op = OperationId::new();
    let relative = "notes/existing.md";
    let (journal, _cache, vault, doc) = staged(
        &core, &temp.path().join("appdata"), relative, b"expected", op,
    );
    m1c_claim(&journal, op);
    let stage = root.join("notes").join(format!(".akula-sync-stage-{op}"));
    let final_name = root.join(relative);
    fs::write(&stage, b"expected").unwrap();
    fs::write(&final_name, b"not from this operation").unwrap();
    let host = RegisteredReadAdmission::open(&core, &root).unwrap();
    let evidence = host.inspect_local_publication(op, &journal).unwrap();
    assert_eq!(evidence.assessment,
        local_recovery::LocalRecoveryAssessment::DifferentFinalFile);
    fs::remove_file(&final_name).unwrap();
    let both_missing_final = host.inspect_local_publication(op, &journal).unwrap();
    assert_eq!(both_missing_final.assessment,
        local_recovery::LocalRecoveryAssessment::StagedOnly);
    fs::write(&stage, b"maliciously replaced").unwrap();
    let corrupted = host.inspect_local_publication(op, &journal).unwrap();
    assert_eq!(corrupted.assessment,
        local_recovery::LocalRecoveryAssessment::UntrustedStage);
    assert_eq!(fs::read(&stage).unwrap(), b"maliciously replaced");
    assert!(journal.baseline(vault, doc).unwrap().is_none());
}

#[test]
fn m1c_both_names_present_remain_ambiguous_even_with_matching_bytes() {
    let (temp, core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let op = OperationId::new();
    let relative = "notes/ambiguous.md";
    let (journal, _cache, vault, doc) = staged(
        &core, &temp.path().join("appdata"), relative, b"same", op,
    );
    m1c_claim(&journal, op);
    let stage = root.join("notes").join(format!(".akula-sync-stage-{op}"));
    fs::write(&stage, b"same").unwrap();
    fs::write(root.join(relative), b"same").unwrap();
    let evidence = RegisteredReadAdmission::open(&core, &root).unwrap()
        .inspect_local_publication(op, &journal).unwrap();
    assert_eq!(evidence.assessment,
        local_recovery::LocalRecoveryAssessment::BothNamesPresent);
    assert_ne!(
        match evidence.stage {
            local_recovery::ObservedLocalFile::Present { identity, .. } => identity,
            _ => panic!("expected staged file"),
        },
        match evidence.final_name {
            local_recovery::ObservedLocalFile::Present { identity, .. } => identity,
            _ => panic!("expected final file"),
        },
    );
    assert!(stage.exists());
    assert_eq!(journal.local_publication_review_queue().unwrap().len(), 1);
    assert!(journal.baseline(vault, doc).unwrap().is_none());
}

#[cfg(unix)]
#[test]
fn m1c_symlinked_stage_and_hardlinked_final_fail_closed() {
    use std::fs::hard_link;
    use std::os::unix::fs::symlink;
    let (temp, core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let external = temp.path().join("external");
    fs::write(&external, b"outside").unwrap();
    let op = OperationId::new();
    let (journal, _cache, vault, doc) = staged(
        &core, &temp.path().join("appdata"), "notes/alias.md", b"expected", op,
    );
    m1c_claim(&journal, op);
    let stage = root.join("notes").join(format!(".akula-sync-stage-{op}"));
    symlink(&external, &stage).unwrap();
    let host = RegisteredReadAdmission::open(&core, &root).unwrap();
    assert!(host.inspect_local_publication(op, &journal).is_err());
    fs::remove_file(stage).unwrap();
    hard_link(&external, root.join("notes/alias.md")).unwrap();
    assert!(host.inspect_local_publication(op, &journal).is_err());
    assert_eq!(fs::read(external).unwrap(), b"outside");
    assert!(journal.baseline(vault, doc).unwrap().is_none());
}

#[test]
fn m1c_detects_same_content_file_id_swap_between_two_reads() {
    let (temp, core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let op = OperationId::new();
    let relative = "notes/swapped.md";
    let (journal, _cache, _, _) = staged(
        &core, &temp.path().join("appdata"), relative, b"same", op,
    );
    m1c_claim(&journal, op);
    let dest = root.join(relative);
    fs::write(&dest, b"same").unwrap();
    let host = RegisteredReadAdmission::open(&core, &root).unwrap();
    let result = host.inspect_local_publication_with_hook(op, &journal, || {
        fs::rename(&dest, root.join("notes/old-id.md")).unwrap();
        fs::write(&dest, b"same").unwrap();
    });
    assert!(matches!(result, Err(LocalRecoveryError::UnstableObservation)));
    assert_eq!(fs::read(root.join("notes/old-id.md")).unwrap(), b"same");
    assert_eq!(fs::read(dest).unwrap(), b"same");
}

#[cfg(unix)]
#[test]
fn m1c_detached_parent_does_not_appear_verified_after_symlink_swap() {
    use std::os::unix::fs::symlink;
    let (temp, core, root) = fixture();
    let parent = root.join("notes");
    fs::create_dir(&parent).unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let op = OperationId::new();
    let (journal, _cache, _, _) = staged(
        &core, &temp.path().join("appdata"), "notes/new.md", b"bytes", op,
    );
    m1c_claim(&journal, op);
    let host = RegisteredReadAdmission::open(&core, &root).unwrap();
    let old = temp.path().join("original-notes");
    let result = host.inspect_local_publication_with_hook(op, &journal, || {
        fs::rename(&parent, &old).unwrap();
        symlink(&outside, &parent).unwrap();
    });
    assert!(matches!(result, Err(LocalRecoveryError::UnstableObservation)));
    assert!(!outside.join("new.md").exists());
}

#[test]
fn m1c_refuses_unknown_or_foreign_vault_ticket_and_oversize_file() {
    let (temp, core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let op = OperationId::new();
    let relative = "notes/large.md";
    let (journal, _cache, _, _) = staged(
        &core, &temp.path().join("appdata"), relative, b"tiny", op,
    );
    let host = RegisteredReadAdmission::open(&core, &root).unwrap();
    assert!(matches!(host.inspect_local_publication(op, &journal),
        Err(LocalRecoveryError::MissingTicket)));
    m1c_claim(&journal, op);
    let other = tempfile::tempdir().unwrap();
    let other_root = other.path().join("vault");
    fs::create_dir(&other_root).unwrap();
    let other_core = core_at(&other_root, &other.path().join("appdata"));
    let other_host = RegisteredReadAdmission::open(&other_core, &other_root).unwrap();
    assert!(other_host.inspect_local_publication(op, &journal).is_err());
    let file = std::fs::OpenOptions::new().create_new(true).write(true)
        .open(root.join(relative)).unwrap();
    file.set_len(32 * 1024 * 1024 + 1).unwrap();
    assert!(matches!(host.inspect_local_publication(op, &journal),
        Err(LocalRecoveryError::Admission(VaultPortError::PermissionDenied))));
}


#[test]
fn m1d_post_crash_review_evidence_is_immutable_across_restarts() {
    let (temp, _core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let op = OperationId::new();
    let relative = "notes/recover.md";
    let output = run_child(&temp, &root, relative, op, "crash");
    assert_eq!(output.status.code(), Some(87),
        "child did not reach the crash point: {}",
        String::from_utf8_lossy(&output.stderr));
    let appdata = temp.path().join("child-appdata");
    let journal = SqliteSyncJournal::open(&appdata.join("ops.sqlite")).unwrap();
    let core = core_at(&root, &appdata);
    let host = RegisteredReadAdmission::open(&core, &root).unwrap();
    let recorded = host.record_local_recovery_evidence(op, &journal).unwrap();
    assert_eq!(recorded.evidence.assessment,
        local_recovery::LocalRecoveryAssessment::StagedOnly);
    assert_eq!(journal.read_local_recovery_case(op).unwrap(), Some(recorded.clone()));
    let record = journal.get(op).unwrap().unwrap();
    assert_eq!(record.phase, JournalPhase::Staged);
    assert!(journal.baseline(record.intent.vault_id, record.intent.document_id)
        .unwrap().is_none());

    let staged_path = root.join("notes").join(format!(".akula-sync-stage-{op}"));
    let moved = temp.path().join("outside-old-stage");
    fs::rename(&staged_path, &moved).unwrap();
    fs::write(&staged_path, b"staged before process exit").unwrap();
    // Identical bytes do not give the replacement the prior OS file ID or
    // authority to rewrite the immutable historical incident.
    assert!(matches!(
        host.record_local_recovery_evidence(op, &journal),
        Err(LocalRecoveryError::Journal(
            JournalError::RecoveryEvidenceAlreadyRecorded))
    ));
    assert_eq!(journal.read_local_recovery_case(op).unwrap(), Some(recorded));
    assert_eq!(fs::read(&moved).unwrap(), b"staged before process exit");
    assert_eq!(fs::read(&staged_path).unwrap(), b"staged before process exit");
    assert!(!root.join(relative).exists());
}

#[test]
fn m1d_exit_after_atomic_publish_still_requires_review_not_baseline() {
    let (temp, _core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let op = OperationId::new();
    let relative = "notes/lost-confirmation.md";
    let output = run_child(&temp, &root, relative, op, "published");
    assert_eq!(output.status.code(), Some(88),
        "child did not exit after no-replace publish: {}",
        String::from_utf8_lossy(&output.stderr));
    assert_eq!(fs::read(root.join(relative)).unwrap(),
        b"published before lost acknowledgement");
    let appdata = temp.path().join("child-appdata");
    let core = core_at(&root, &appdata);
    let journal = SqliteSyncJournal::open(&appdata.join("ops.sqlite")).unwrap();
    let host = RegisteredReadAdmission::open(&core, &root).unwrap();
    let case = host.record_local_recovery_evidence(op, &journal).unwrap();
    assert_eq!(case.evidence.assessment,
        local_recovery::LocalRecoveryAssessment::FinalNameMatchesExpected);
    assert_eq!(journal.get(op).unwrap().unwrap().phase, JournalPhase::Staged);
    let doc = journal.get(op).unwrap().unwrap().intent;
    assert!(journal.baseline(doc.vault_id, doc.document_id).unwrap().is_none());
    assert!(matches!(
        journal.read_local_recovery_case(op),
        Ok(Some(_))
    ));
    // The only supported disposition remains HOLD: a matching final inode
    // does not prove it survived an actual power loss or was created by us.
    assert!(!root.join("notes").join(format!(".akula-sync-stage-{op}")).exists());
}

#[test]
fn m1d_no_ticket_cannot_persist_recovery_evidence() {
    let (temp, core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let op = OperationId::new();
    let (journal, _cache, vault, doc) = staged(
        &core, &temp.path().join("appdata"), "notes/unclaimed.md", b"new", op,
    );
    let host = RegisteredReadAdmission::open(&core, &root).unwrap();
    assert!(matches!(
        host.record_local_recovery_evidence(op, &journal),
        Err(LocalRecoveryError::MissingTicket)
    ));
    assert!(journal.read_local_recovery_case(op).unwrap().is_none());
    assert!(journal.baseline(vault, doc).unwrap().is_none());
}


#[test]
fn m1db_crash_stage_copy_is_retained_after_original_loss_and_restart() {
    let (temp, _core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let op = OperationId::new();
    let relative = "notes/crash-stage.md";
    let output = run_child(&temp, &root, relative, op, "crash");
    assert_eq!(output.status.code(), Some(87));
    let appdata = temp.path().join("child-appdata");
    let core = core_at(&root, &appdata);
    let db = appdata.join("ops.sqlite");
    let journal = SqliteSyncJournal::open(&db).unwrap();
    let host = RegisteredReadAdmission::open(&core, &root).unwrap();
    let case = host.record_local_recovery_evidence(op, &journal).unwrap();
    assert_eq!(case.evidence.assessment,
        local_recovery::LocalRecoveryAssessment::StagedOnly);
    let archive = host.archive_local_recovery_copy(op, &journal).unwrap();
    assert_eq!(archive.stage_bytes.as_deref(), Some(&b"staged before process exit"[..]));
    assert_eq!(archive.final_bytes, None);
    let incident = journal.get(op).unwrap().unwrap().intent;
    assert!(journal.baseline(incident.vault_id, incident.document_id).unwrap().is_none());
    assert!(matches!(host.archive_local_recovery_copy(op, &journal),
        Err(LocalRecoveryError::Journal(JournalError::RecoveryCopyAlreadyRecorded))));
    let stage = root.join("notes").join(format!(".akula-sync-stage-{op}"));
    fs::remove_file(&stage).unwrap(); // simulate external loss, NOT product recovery
    drop(journal);
    let reopened = SqliteSyncJournal::open(&db).unwrap();
    assert_eq!(reopened.read_local_recovery_copy(op).unwrap(), Some(archive));
    assert!(!stage.exists());
    assert_eq!(reopened.local_publication_review_queue().unwrap().len(), 1);
    assert!(reopened.baseline(incident.vault_id, incident.document_id)
        .unwrap().is_none());
}

#[test]
fn m1db_published_file_copy_does_not_mutate_vault_or_confirm_baseline() {
    let (temp, _core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let op = OperationId::new();
    let relative = "notes/lost-ack.md";
    let output = run_child(&temp, &root, relative, op, "published");
    assert_eq!(output.status.code(), Some(88));
    let appdata = temp.path().join("child-appdata");
    let core = core_at(&root, &appdata);
    let journal = SqliteSyncJournal::open(&appdata.join("ops.sqlite")).unwrap();
    let host = RegisteredReadAdmission::open(&core, &root).unwrap();
    host.record_local_recovery_evidence(op, &journal).unwrap();
    let archive = host.archive_local_recovery_copy(op, &journal).unwrap();
    assert_eq!(archive.stage_bytes, None);
    assert_eq!(archive.final_bytes.as_deref(),
        Some(&b"published before lost acknowledgement"[..]));
    assert_eq!(fs::read(root.join(relative)).unwrap(),
        b"published before lost acknowledgement");
    let intent = journal.get(op).unwrap().unwrap().intent;
    assert_eq!(journal.get(op).unwrap().unwrap().phase, JournalPhase::Staged);
    assert!(journal.baseline(intent.vault_id, intent.document_id).unwrap().is_none());
}

#[test]
fn m1db_external_stage_inode_swap_with_same_bytes_refuses_archive() {
    let (temp, core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let op = OperationId::new();
    let relative = "notes/external.md";
    let (journal, _cache, vault, doc) = staged(
        &core, &temp.path().join("appdata"), relative, b"unchanged bytes", op,
    );
    m1c_claim(&journal, op);
    let stage = root.join("notes").join(format!(".akula-sync-stage-{op}"));
    fs::write(&stage, b"unchanged bytes").unwrap();
    let host = RegisteredReadAdmission::open(&core, &root).unwrap();
    host.record_local_recovery_evidence(op, &journal).unwrap();
    let moved = temp.path().join("original-stage");
    fs::rename(&stage, &moved).unwrap();
    fs::write(&stage, b"unchanged bytes").unwrap();
    assert!(host.archive_local_recovery_copy(op, &journal).is_err());
    assert!(journal.read_local_recovery_copy(op).unwrap().is_none());
    assert_eq!(fs::read(&moved).unwrap(), b"unchanged bytes");
    assert_eq!(fs::read(&stage).unwrap(), b"unchanged bytes");
    assert!(journal.baseline(vault, doc).unwrap().is_none());
}

#[test]
fn m1db_conflicting_external_final_bytes_are_copied_not_overwritten() {
    let (temp, core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let op = OperationId::new();
    let relative = "notes/competing.md";
    let (journal, _cache, vault, doc) = staged(
        &core, &temp.path().join("appdata"), relative, b"our intended data", op,
    );
    m1c_claim(&journal, op);
    let stage = root.join("notes").join(format!(".akula-sync-stage-{op}"));
    fs::write(&stage, b"our intended data").unwrap();
    fs::write(root.join(relative), b"other process data").unwrap();
    let host = RegisteredReadAdmission::open(&core, &root).unwrap();
    let evidence = host.record_local_recovery_evidence(op, &journal).unwrap();
    assert_eq!(evidence.evidence.assessment,
        local_recovery::LocalRecoveryAssessment::DifferentFinalFile);
    let copy = host.archive_local_recovery_copy(op, &journal).unwrap();
    assert_eq!(copy.stage_bytes.as_deref(), Some(&b"our intended data"[..]));
    assert_eq!(copy.final_bytes.as_deref(), Some(&b"other process data"[..]));
    assert_eq!(fs::read(&stage).unwrap(), b"our intended data");
    assert_eq!(fs::read(root.join(relative)).unwrap(), b"other process data");
    assert!(journal.baseline(vault, doc).unwrap().is_none());
}

#[test]
fn m1db_refuses_appdata_archive_inside_registered_vault() {
    let (temp, core, root) = fixture();
    fs::create_dir_all(root.join("notes")).unwrap();
    let op = OperationId::new();
    let (journal, _cache, vault, doc) = staged(
        &core, &root.join("untrusted-journal"), "notes/user.md", b"payload", op,
    );
    m1c_claim(&journal, op);
    let host = RegisteredReadAdmission::open(&core, &root).unwrap();
    // A read-only inspection can describe an incident, but writing either a
    // forensic case or byte archive to a SQLite file *inside* the vault must
    // fail closed, even when the bytes themselves were not changed.
    let _ = host.inspect_local_publication(op, &journal).unwrap();
    assert!(matches!(
        host.record_local_recovery_evidence(op, &journal),
        Err(LocalRecoveryError::Journal(JournalError::UnsafeArchiveLocation))
    ));
    assert!(journal.read_local_recovery_case(op).unwrap().is_none());
    assert!(matches!(
        host.archive_local_recovery_copy(op, &journal),
        Err(LocalRecoveryError::Journal(JournalError::UnsafeArchiveLocation))
    ));
    assert!(journal.read_local_recovery_copy(op).unwrap().is_none());
    assert!(journal.baseline(vault, doc).unwrap().is_none());
}


#[test]
fn m1dc_explicit_review_hold_then_escalation_never_changes_files_or_baseline() {
    use super::super::recovery_review::{RecoveryReviewDisposition, RECOVERY_REVIEW_RETENTION};
    let (temp, core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let op = OperationId::new();
    let relative = "notes/manual-review.md";
    let (mut journal, _cache, vault, doc) = staged(
        &core, &temp.path().join("appdata"), relative, b"safe staged bytes", op,
    );
    m1c_claim(&journal, op);
    let stage = root.join("notes").join(format!(".akula-sync-stage-{op}"));
    let final_file = root.join(relative);
    fs::write(&stage, b"safe staged bytes").unwrap();
    fs::write(&final_file, b"different human work").unwrap();
    let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
    admission.record_local_recovery_evidence(op, &journal).unwrap();
    admission.archive_local_recovery_copy(op, &journal).unwrap();

    let preview = admission.recovery_operator_preview(op, &journal).unwrap();
    assert_eq!(preview.retention, RECOVERY_REVIEW_RETENTION);
    assert_eq!(preview.review_count, 0);
    assert_eq!(preview.latest_event_digest, "genesis");
    assert_eq!(preview.assessment,
        local_recovery::LocalRecoveryAssessment::DifferentFinalFile);
    let first = admission.record_recovery_operator_review(
        op, &preview.path, &preview.archive_fingerprint,
        preview.review_count, &preview.latest_event_digest,
        RecoveryReviewDisposition::KeepBlocked,
        "Keep both original versions for inspection", &mut journal,
    ).unwrap();
    assert_eq!(first.sequence, 1);
    assert_eq!(first.disposition, RecoveryReviewDisposition::KeepBlocked);

    let new_preview = admission.recovery_operator_preview(op, &journal).unwrap();
    assert_eq!(new_preview.review_count, 1);
    assert_ne!(new_preview.latest_event_digest, "genesis");
    let second = admission.record_recovery_operator_review(
        op, &new_preview.path, &new_preview.archive_fingerprint,
        new_preview.review_count, &new_preview.latest_event_digest,
        RecoveryReviewDisposition::EscalateManualReview,
        "Requires verified Windows durability before future resolution",
        &mut journal,
    ).unwrap();
    assert_eq!(second.sequence, 2);
    assert_eq!(second.previous_event_digest, new_preview.latest_event_digest);
    let history = journal.recovery_review_history(op).unwrap();
    assert_eq!(history, vec![first, second]);
    assert_eq!(fs::read(&stage).unwrap(), b"safe staged bytes");
    assert_eq!(fs::read(&final_file).unwrap(), b"different human work");
    assert!(journal.baseline(vault, doc).unwrap().is_none());
    assert_eq!(journal.local_publication_review_queue().unwrap().len(), 1);
    assert_eq!(journal.get(op).unwrap().unwrap().phase, JournalPhase::Staged);
    let archive = journal.read_local_recovery_copy(op).unwrap().unwrap();
    assert_eq!(archive.stage_bytes.as_deref(), Some(&b"safe staged bytes"[..]));
    assert_eq!(archive.final_bytes.as_deref(), Some(&b"different human work"[..]));
}

#[test]
fn m1dc_stale_or_forged_confirmation_is_rejected_without_new_review() {
    use super::super::recovery_review::RecoveryReviewDisposition;
    let (temp, core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let op = OperationId::new();
    let (mut journal, _cache, vault, doc) = staged(
        &core, &temp.path().join("appdata"), "notes/for-review.md", b"payload", op,
    );
    m1c_claim(&journal, op);
    let stage = root.join("notes").join(format!(".akula-sync-stage-{op}"));
    fs::write(&stage, b"payload").unwrap();
    let host = RegisteredReadAdmission::open(&core, &root).unwrap();
    host.record_local_recovery_evidence(op, &journal).unwrap();
    let no_archive = host.recovery_operator_preview(op, &journal);
    assert!(matches!(
        no_archive, Err(LocalRecoveryError::Journal(JournalError::MissingRecoveryArchive))
    ));
    host.archive_local_recovery_copy(op, &journal).unwrap();
    let preview = host.recovery_operator_preview(op, &journal).unwrap();
    let wrong = SyncRelativePath::parse("notes/wrong.md").unwrap();
    for (path, hash, count, prev, note) in [
        (&wrong, preview.archive_fingerprint.as_str(), 0, "genesis", "valid"),
        (&preview.path, "deadbeef", 0, "genesis", "valid"),
        (&preview.path, preview.archive_fingerprint.as_str(), 1, "genesis", "valid"),
        (&preview.path, preview.archive_fingerprint.as_str(), 0, "outdated", "valid"),
    ] {
        assert!(matches!(
            host.record_recovery_operator_review(
                op, path, hash, count, prev,
                RecoveryReviewDisposition::KeepBlocked, note, &mut journal,
            ),
            Err(LocalRecoveryError::Journal(JournalError::StaleRecoveryReview))
        ));
    }
    for note in ["  ", "contains\nnewline", "leading\ttab"] {
        assert!(matches!(
            host.record_recovery_operator_review(
                op, &preview.path, &preview.archive_fingerprint,
                0, "genesis", RecoveryReviewDisposition::KeepBlocked,
                note, &mut journal,
            ),
            Err(LocalRecoveryError::Journal(JournalError::InvalidRecoveryReview))
        ));
    }
    assert!(journal.recovery_review_history(op).unwrap().is_empty());
    assert_eq!(fs::read(&stage).unwrap(), b"payload");
    assert!(journal.baseline(vault, doc).unwrap().is_none());
}

#[test]
fn m1dc_review_requires_registered_host_and_archive_outside_vault() {
    use super::super::recovery_review::RecoveryReviewDisposition;
    let (temp, core, root) = fixture();
    fs::create_dir(root.join("notes")).unwrap();
    let op = OperationId::new();
    let (mut journal, _cache, vault, doc) = staged(
        &core, &temp.path().join("appdata"), "notes/hold.md", b"payload", op,
    );
    m1c_claim(&journal, op);
    fs::write(root.join("notes").join(format!(".akula-sync-stage-{op}")),
        b"payload").unwrap();
    let host = RegisteredReadAdmission::open(&core, &root).unwrap();
    host.record_local_recovery_evidence(op, &journal).unwrap();
    host.archive_local_recovery_copy(op, &journal).unwrap();
    let preview = host.recovery_operator_preview(op, &journal).unwrap();
    let other = tempfile::tempdir().unwrap();
    let other_root = other.path().join("vault");
    fs::create_dir(&other_root).unwrap();
    let other_core = core_at(&other_root, &other.path().join("appdata"));
    let other_host = RegisteredReadAdmission::open(&other_core, &other_root).unwrap();
    assert!(other_host.recovery_operator_preview(op, &journal).is_err());
    assert!(other_host.record_recovery_operator_review(
        op, &preview.path, &preview.archive_fingerprint, 0, "genesis",
        RecoveryReviewDisposition::KeepBlocked, "not my vault", &mut journal,
    ).is_err());
    assert!(journal.recovery_review_history(op).unwrap().is_empty());
    assert!(journal.baseline(vault, doc).unwrap().is_none());
}

#[cfg(feature = "host-custody")]
#[test]
fn mandatory_host_epoch_refuses_internal_staged_create_and_recovery_copy() {
    // A normal Core may still exercise the opt-in additive experiment.
    // An explicitly installed-host Core lacks a shared enrollment epoch for
    // this internal method, even though it takes the process File Gate mutex.
    // Previously this was an alternate path around Core's mandatory HOLD.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let appdata = temp.path().join("appdata");
    let hostdir = temp.path().join("installer-host");
    for d in [&root, &appdata, &hostdir] {
        fs::create_dir(d).unwrap();
    }
    let root = fs::canonicalize(&root).unwrap();
    fs::create_dir(root.join("notes")).unwrap();
    let outside_db = hostdir.join("approved-host.sqlite3");
    let core = Core::open_with_installed_host(
        &appdata, &outside_db, Arc::new(|_: CoreEvent| {}),
    );
    assert!(core.requires_installed_host_epoch());
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();

    let operation = OperationId::new();
    let (journal, cache, vault, doc) = staged(
        &core, &appdata, "notes/Protected.md", b"do not publish", operation,
    );
    let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
    assert!(matches!(
        admission.create_staged_if_absent(operation, &journal, &cache),
        Err(LocalCreateError::Admission(VaultPortError::PermissionDenied))
    ), "staged local create escaped mandatory shared-host HOLD");
    assert!(!root.join("notes/Protected.md").exists());
    assert_eq!(journal.get(operation).unwrap().unwrap().phase, JournalPhase::Staged);
    assert!(journal.baseline(vault, doc).unwrap().is_none());

    // Recovery archive is another writer that does not currently retain a
    // shared host epoch. It must refuse at admission, not after a copy.
    assert!(matches!(
        admission.archive_local_recovery_copy_for_vault(operation, &journal, vault),
        Err(LocalRecoveryError::Admission(VaultPortError::PermissionDenied))
    ), "internal recovery archive escaped mandatory shared-host HOLD");
    assert!(!root.join("notes/Protected.md").exists());
    assert!(!outside_db.exists(),
        "a refusal must not bootstrap its own valid-empty host ledger");
}
