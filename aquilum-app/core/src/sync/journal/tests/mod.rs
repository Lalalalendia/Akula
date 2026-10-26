use super::*;

fn intent(
    vault: VaultId,
    doc: DocumentId,
    revision: RevisionId,
    parent: Option<RevisionId>,
    bytes: &[u8],
) -> OperationIntent {
    OperationIntent {
        operation_id: OperationId::new(),
        vault_id: vault,
        document_id: doc,
        profile_id: "s3-profile-1".into(),
        revision_id: revision,
        action: JournalAction::UploadImmutable,
        path: SyncRelativePath::parse("notes/demo.md").unwrap(),
        remote_object_id: format!("immutable/{}", revision),
        remote_guard: Some(RemoteGuard::Absent),
        expected_content_hash: blake3::hash(bytes).to_hex().to_string(),
        expected_baseline: parent,
    }
}

#[test]
fn commit_requires_staging_application_verified_readback_and_atomic_baseline() {
    let mut j = SqliteSyncJournal::memory().unwrap();
    let vault = VaultId::new();
    let doc = DocumentId::new();
    let rev = RevisionId::new();
    let op = intent(vault, doc, rev, None, b"contents");
    let id = op.operation_id;
    assert_eq!(j.begin(op.clone()).unwrap(), JournalPhase::Pending);
    assert!(j.baseline(vault, doc).unwrap().is_none());
    assert!(matches!(j.commit(id), Err(JournalError::InvalidTransition)));
    assert_eq!(
        j.mark_staged(id, b"contents").unwrap(),
        JournalPhase::Staged
    );
    assert!(matches!(j.commit(id), Err(JournalError::InvalidTransition)));
    assert_eq!(j.mark_applied(id).unwrap(), JournalPhase::Applied);
    assert!(matches!(
        j.mark_verified(id, b"corruption", "provider:1"),
        Err(JournalError::ReadbackMismatch)
    ));
    assert!(j.baseline(vault, doc).unwrap().is_none());
    assert_eq!(
        j.mark_verified(id, b"contents", "provider:1").unwrap(),
        JournalPhase::Verified
    );
    assert!(j.baseline(vault, doc).unwrap().is_none());
    assert_eq!(j.commit(id).unwrap(), JournalPhase::Committed);
    assert_eq!(j.commit(id).unwrap(), JournalPhase::Committed);
    assert_eq!(j.baseline(vault, doc).unwrap().unwrap().revision_id, rev);
    assert!(j.recovery_queue().unwrap().is_empty());
}

#[test]
fn duplicate_operation_is_idempotent_and_uuid_collision_rejected() {
    let mut j = SqliteSyncJournal::memory().unwrap();
    let vault = VaultId::new();
    let doc = DocumentId::new();
    let rev = RevisionId::new();
    let op = intent(vault, doc, rev, None, b"contents");
    assert_eq!(j.begin(op.clone()).unwrap(), JournalPhase::Pending);
    assert_eq!(j.begin(op.clone()).unwrap(), JournalPhase::Pending);
    let mut tampered = op;
    tampered.remote_object_id = "another-object".into();
    assert!(matches!(
        j.begin(tampered),
        Err(JournalError::IntentCollision)
    ));
}

#[test]
fn two_workers_cannot_commit_simultaneously_to_one_document() {
    let mut j = SqliteSyncJournal::memory().unwrap();
    let vault = VaultId::new();
    let doc = DocumentId::new();
    let first = intent(vault, doc, RevisionId::new(), None, b"first");
    let second = intent(vault, doc, RevisionId::new(), None, b"second");
    assert_eq!(j.begin(first.clone()).unwrap(), JournalPhase::Pending);
    assert!(matches!(
        j.begin(second.clone()),
        Err(JournalError::BusyDocument)
    ));
    j.mark_staged(first.operation_id, b"first").unwrap();
    j.mark_applied(first.operation_id).unwrap();
    j.mark_verified(first.operation_id, b"first", "rev-1")
        .unwrap();
    j.commit(first.operation_id).unwrap();
    assert!(matches!(
        j.begin(second.clone()),
        Err(JournalError::BaselineChanged)
    ));

    let third = intent(
        vault,
        doc,
        RevisionId::new(),
        Some(first.revision_id),
        b"third",
    );
    assert_eq!(j.begin(third).unwrap(), JournalPhase::Pending);
}

#[test]
fn process_restarts_never_auto_reapply_uncertain_side_effects() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sync-actions.sqlite");
    let vault = VaultId::new();
    let doc = DocumentId::new();
    let a = intent(vault, doc, RevisionId::new(), None, b"a");
    let b = intent(vault, DocumentId::new(), RevisionId::new(), None, b"b");
    let c = intent(vault, DocumentId::new(), RevisionId::new(), None, b"c");
    let d = intent(vault, DocumentId::new(), RevisionId::new(), None, b"d");
    {
        let mut j = SqliteSyncJournal::open(&path).unwrap();
        j.begin(a.clone()).unwrap();
        j.begin(b.clone()).unwrap();
        j.begin(c.clone()).unwrap();
        j.begin(d.clone()).unwrap();
        j.mark_staged(b.operation_id, b"b").unwrap();
        j.mark_staged(c.operation_id, b"c").unwrap();
        j.mark_applied(c.operation_id).unwrap();
        j.mark_staged(d.operation_id, b"d").unwrap();
        j.mark_applied(d.operation_id).unwrap();
        j.mark_verified(d.operation_id, b"d", "rev-d").unwrap();
    }
    let j = SqliteSyncJournal::open(&path).unwrap();
    let recovery = j.recovery_queue().unwrap();
    assert_eq!(recovery.len(), 4);
    assert_eq!(
        recovery
            .iter()
            .find(|x| x.record.intent.operation_id == a.operation_id)
            .unwrap()
            .action,
        RecoveryAction::ResumeStaging
    );
    assert_eq!(
        recovery
            .iter()
            .find(|x| x.record.intent.operation_id == b.operation_id)
            .unwrap()
            .action,
        RecoveryAction::InspectBeforeRetry
    );
    assert_eq!(
        recovery
            .iter()
            .find(|x| x.record.intent.operation_id == c.operation_id)
            .unwrap()
            .action,
        RecoveryAction::InspectBeforeRetry
    );
    assert_eq!(
        recovery
            .iter()
            .find(|x| x.record.intent.operation_id == d.operation_id)
            .unwrap()
            .action,
        RecoveryAction::RetryCommit
    );
    assert!(j.baseline(vault, doc).unwrap().is_none());
}

#[test]
fn sqlite_failure_rolls_back_baseline_and_final_state_together() {
    let mut j = SqliteSyncJournal::memory().unwrap();
    let vault = VaultId::new();
    let doc = DocumentId::new();
    let op = intent(vault, doc, RevisionId::new(), None, b"verify");
    j.begin(op.clone()).unwrap();
    j.mark_staged(op.operation_id, b"verify").unwrap();
    j.mark_applied(op.operation_id).unwrap();
    j.mark_verified(op.operation_id, b"verify", "provider:1")
        .unwrap();
    j.connection
        .execute_batch(
            "CREATE TRIGGER simulated_commit_failure BEFORE UPDATE ON sync_operations
             WHEN NEW.phase = 4 BEGIN SELECT RAISE(FAIL, 'power-loss simulation'); END;",
        )
        .unwrap();
    assert!(matches!(
        j.commit(op.operation_id),
        Err(JournalError::Sqlite(_))
    ));
    assert!(j.baseline(vault, doc).unwrap().is_none());
    assert_eq!(
        j.get(op.operation_id).unwrap().unwrap().phase,
        JournalPhase::Verified
    );
    j.connection
        .execute_batch("DROP TRIGGER simulated_commit_failure;")
        .unwrap();
    assert_eq!(j.commit(op.operation_id).unwrap(), JournalPhase::Committed);
    assert_eq!(
        j.baseline(vault, doc).unwrap().unwrap().revision_id,
        op.revision_id
    );
}

#[test]
fn sqlite_corrupted_intent_or_baseline_fails_closed() {
    let mut j = SqliteSyncJournal::memory().unwrap();
    let vault = VaultId::new();
    let doc = DocumentId::new();
    let op = intent(vault, doc, RevisionId::new(), None, b"original");
    j.begin(op.clone()).unwrap();
    j.connection
        .execute(
            "UPDATE sync_operations SET intent = ?1 WHERE operation_id = ?2",
            params![b"broken".as_slice(), op.operation_id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        j.get(op.operation_id),
        Err(JournalError::CorruptedRecord)
    ));
}

#[test]
fn wrong_hash_and_illegal_phase_transitions_never_advance_state() {
    let mut j = SqliteSyncJournal::memory().unwrap();
    let op = intent(
        VaultId::new(),
        DocumentId::new(),
        RevisionId::new(),
        None,
        b"valid",
    );
    j.begin(op.clone()).unwrap();
    assert!(matches!(
        j.mark_staged(op.operation_id, b"invalid"),
        Err(JournalError::ReadbackMismatch)
    ));
    assert!(matches!(
        j.mark_applied(op.operation_id),
        Err(JournalError::InvalidTransition)
    ));
    assert!(matches!(
        j.mark_verified(op.operation_id, b"valid", "etag"),
        Err(JournalError::InvalidTransition)
    ));
    assert_eq!(
        j.get(op.operation_id).unwrap().unwrap().phase,
        JournalPhase::Pending
    );
}

#[test]
fn implicit_remote_overwrite_and_profile_changes_fail_closed() {
    let mut j = SqliteSyncJournal::memory().unwrap();
    let vault = VaultId::new();
    let document = DocumentId::new();
    let mut bad = intent(vault, document, RevisionId::new(), None, b"data");
    bad.remote_guard = None;
    assert!(matches!(
        j.begin(bad.clone()),
        Err(JournalError::InvalidIntent)
    ));
    bad.remote_guard = Some(RemoteGuard::Matches("etag-1".into()));
    assert!(matches!(j.begin(bad), Err(JournalError::InvalidIntent)));
    let mut first = intent(vault, document, RevisionId::new(), None, b"one");
    first.action = JournalAction::PublishHead;
    first.remote_guard = Some(RemoteGuard::Matches("etag-1".into()));
    j.begin(first.clone()).unwrap();
    j.mark_staged(first.operation_id, b"one").unwrap();
    j.mark_applied(first.operation_id).unwrap();
    j.mark_verified(first.operation_id, b"one", "etag-2")
        .unwrap();
    j.commit(first.operation_id).unwrap();
    let mut different_profile = intent(
        vault,
        document,
        RevisionId::new(),
        Some(first.revision_id),
        b"two",
    );
    different_profile.profile_id = "google-drive-different-root".into();
    assert!(matches!(
        j.begin(different_profile),
        Err(JournalError::ProfileChanged)
    ));
}

fn local_stage_intent(
    vault: VaultId,
    document: DocumentId,
    path: &str,
    bytes: &[u8],
) -> OperationIntent {
    let mut item = intent(vault, document, RevisionId::new(), None, bytes);
    item.action = JournalAction::StageDownload;
    item.remote_guard = None;
    item.path = SyncRelativePath::parse(path).unwrap();
    item
}

#[test]
fn local_publication_claim_is_once_per_operation_and_portable_name() {
    let mut journal = SqliteSyncJournal::memory().unwrap();
    let vault = VaultId::new();
    let first = local_stage_intent(vault, DocumentId::new(), "notes/Café.md", b"one");
    let second = local_stage_intent(vault, DocumentId::new(), "notes/cafe\u{301}.md", b"two");
    journal.begin(first.clone()).unwrap();
    journal.begin(second.clone()).unwrap();
    journal.mark_staged(first.operation_id, b"one").unwrap();
    journal.mark_staged(second.operation_id, b"two").unwrap();

    journal
        .record_local_publication_attempt_once(&first)
        .unwrap();
    assert!(matches!(
        journal.record_local_publication_attempt_once(&first),
        Err(JournalError::PublicationAttemptAlreadyRecorded)
    ));
    assert!(matches!(
        journal.record_local_publication_attempt_once(&second),
        Err(JournalError::PublicationAttemptAlreadyRecorded)
    ));
    let queue = journal.local_publication_review_queue().unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].operation_id, first.operation_id);
    assert_eq!(queue[0].path.as_str(), "notes/Café.md");
    assert!(journal
        .baseline(vault, first.document_id)
        .unwrap()
        .is_none());
    assert!(journal
        .baseline(vault, second.document_id)
        .unwrap()
        .is_none());
}

#[test]
fn local_publication_claim_requires_a_verified_staged_intent() {
    let mut journal = SqliteSyncJournal::memory().unwrap();
    let staged = local_stage_intent(
        VaultId::new(),
        DocumentId::new(),
        "notes/new.md",
        b"verified",
    );
    assert!(matches!(
        journal.record_local_publication_attempt_once(&staged),
        Err(JournalError::MissingOperation)
    ));
    journal.begin(staged.clone()).unwrap();
    assert!(matches!(
        journal.record_local_publication_attempt_once(&staged),
        Err(JournalError::InvalidTransition)
    ));
    journal
        .mark_staged(staged.operation_id, b"verified")
        .unwrap();
    let mut collision = staged.clone();
    collision.expected_content_hash = blake3::hash(b"other").to_hex().to_string();
    assert!(matches!(
        journal.record_local_publication_attempt_once(&collision),
        Err(JournalError::IntentCollision)
    ));
    journal
        .record_local_publication_attempt_once(&staged)
        .unwrap();
}

#[test]
fn legacy_schema_one_migrates_without_losing_staged_operations() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sync.sqlite");
    let staged = local_stage_intent(
        VaultId::new(),
        DocumentId::new(),
        "notes/legacy.md",
        b"persist",
    );
    {
        let mut journal = SqliteSyncJournal::open(&path).unwrap();
        journal.begin(staged.clone()).unwrap();
        journal
            .mark_staged(staged.operation_id, b"persist")
            .unwrap();
        // Emulate the exact existing V1 tables/data, without the new table.
        journal
            .connection
            .execute_batch(
                "DROP TABLE sync_local_native_publication_observations;
                 DROP TABLE sync_local_recovery_reviews;
                 DROP TABLE sync_local_recovery_copies;
                 DROP TABLE sync_local_recovery_evidence;
                 DROP TABLE sync_local_publication_attempts;
                 DROP TABLE IF EXISTS sync_existing_replace_receipt_candidates;
                 DROP TABLE IF EXISTS sync_existing_replace_attempts;
                  PRAGMA user_version = 1;",
            )
            .unwrap();
    }
    let migrated = SqliteSyncJournal::open(&path).unwrap();
    let version: i64 = migrated
        .connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    assert_eq!(
        migrated.get(staged.operation_id).unwrap().unwrap().phase,
        JournalPhase::Staged
    );
    migrated
        .record_local_publication_attempt_once(&staged)
        .unwrap();
    drop(migrated);
    let reopened = SqliteSyncJournal::open(&path).unwrap();
    assert!(matches!(
        reopened.record_local_publication_attempt_once(&staged),
        Err(JournalError::PublicationAttemptAlreadyRecorded)
    ));
    let queue = reopened.local_publication_review_queue().unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].expected_content_hash, staged.expected_content_hash);
    assert!(reopened
        .baseline(staged.vault_id, staged.document_id)
        .unwrap()
        .is_none());
}

#[test]
fn local_publication_ledger_corruption_fails_closed() {
    let mut journal = SqliteSyncJournal::memory().unwrap();
    let staged = local_stage_intent(
        VaultId::new(),
        DocumentId::new(),
        "notes/new.md",
        b"payload",
    );
    journal.begin(staged.clone()).unwrap();
    journal
        .mark_staged(staged.operation_id, b"payload")
        .unwrap();
    journal
        .record_local_publication_attempt_once(&staged)
        .unwrap();
    journal
        .connection
        .execute(
            "UPDATE sync_local_publication_attempts
             SET expected_hash = 'corrupted' WHERE operation_id = ?1",
            [staged.operation_id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        journal.local_publication_review_queue(),
        Err(JournalError::CorruptedRecord)
    ));
}

#[test]
fn separate_sqlite_connections_cannot_claim_the_same_local_publication() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sync.sqlite");
    let staged = local_stage_intent(
        VaultId::new(),
        DocumentId::new(),
        "notes/two-processes.md",
        b"claim",
    );
    {
        let mut journal = SqliteSyncJournal::open(&path).unwrap();
        journal.begin(staged.clone()).unwrap();
        journal.mark_staged(staged.operation_id, b"claim").unwrap();
    }
    let first = SqliteSyncJournal::open(&path).unwrap();
    let second = SqliteSyncJournal::open(&path).unwrap();
    first
        .record_local_publication_attempt_once(&staged)
        .unwrap();
    assert!(matches!(
        second.record_local_publication_attempt_once(&staged),
        Err(JournalError::PublicationAttemptAlreadyRecorded)
    ));
    assert_eq!(second.local_publication_review_queue().unwrap().len(), 1);
}

#[test]
fn m1d_local_download_cannot_promote_to_confirmed_baseline() {
    let mut journal = SqliteSyncJournal::memory().unwrap();
    let vault = VaultId::new();
    let document = DocumentId::new();
    let staged = local_stage_intent(vault, document, "notes/remote.md", b"payload");
    journal.begin(staged.clone()).unwrap();
    journal
        .mark_staged(staged.operation_id, b"payload")
        .unwrap();
    journal
        .record_local_publication_attempt_once(&staged)
        .unwrap();
    assert!(matches!(
        journal.mark_applied(staged.operation_id),
        Err(JournalError::LocalDownloadNeedsNativeReceipt)
    ));
    assert!(matches!(
        journal.mark_verified(staged.operation_id, b"payload", "fake-rev"),
        Err(JournalError::LocalDownloadNeedsNativeReceipt)
    ));
    assert!(matches!(
        journal.commit(staged.operation_id),
        Err(JournalError::LocalDownloadNeedsNativeReceipt)
    ));
    assert_eq!(
        journal.get(staged.operation_id).unwrap().unwrap().phase,
        JournalPhase::Staged
    );
    assert!(journal.baseline(vault, document).unwrap().is_none());

    // Older or damaged databases might already contain a false Verified
    // state. The generic commit API must STILL refuse and the recovery
    // queue must not describe it as a safe RetryCommit.
    journal
        .connection
        .execute(
            "UPDATE sync_operations
             SET phase = 3, readback_hash = ?1, provider_revision = 'obsolete'
             WHERE operation_id = ?2",
            params![
                staged.expected_content_hash,
                staged.operation_id.to_string()
            ],
        )
        .unwrap();
    assert!(matches!(
        journal.commit(staged.operation_id),
        Err(JournalError::LocalDownloadNeedsNativeReceipt)
    ));
    assert_eq!(
        journal.recovery_queue().unwrap()[0].action,
        RecoveryAction::InspectBeforeRetry
    );
    assert!(journal.baseline(vault, document).unwrap().is_none());
}

fn m1d_test_evidence(staged: &OperationIntent) -> LocalRecoveryEvidenceCase {
    LocalRecoveryEvidenceCase {
        operation_id: staged.operation_id,
        vault_id: staged.vault_id,
        relative_path: staged.path.clone(),
        expected_content_hash: staged.expected_content_hash.clone(),
        evidence: ReadOnlyLocalPublicationEvidence {
            operation_id: staged.operation_id,
            parent_identity: super::super::PinnedFileIdentity {
                device: 1,
                file: 123,
            },
            stage: ObservedLocalFile::Present {
                identity: super::super::PinnedFileIdentity {
                    device: 1,
                    file: 456,
                },
                byte_length: 7,
                matches_expected_hash: true,
            },
            final_name: ObservedLocalFile::Absent,
            assessment: local_recovery::LocalRecoveryAssessment::StagedOnly,
        },
    }
}

#[test]
fn m1d_recovery_evidence_is_write_once_and_corruption_checked_after_restart() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sync.sqlite");
    let staged = local_stage_intent(
        VaultId::new(),
        DocumentId::new(),
        "notes/recover.md",
        b"payload",
    );
    let original = m1d_test_evidence(&staged);
    {
        let mut journal = SqliteSyncJournal::open(&path).unwrap();
        journal.begin(staged.clone()).unwrap();
        journal
            .mark_staged(staged.operation_id, b"payload")
            .unwrap();
        journal
            .record_local_publication_attempt_once(&staged)
            .unwrap();
        journal.record_local_recovery_case_once(&original).unwrap();
    }
    let reopened = SqliteSyncJournal::open(&path).unwrap();
    assert_eq!(
        reopened
            .read_local_recovery_case(staged.operation_id)
            .unwrap(),
        Some(original.clone())
    );
    assert!(matches!(
        reopened.record_local_recovery_case_once(&original),
        Err(JournalError::RecoveryEvidenceAlreadyRecorded)
    ));
    let mut changed = original.clone();
    changed.evidence.stage = ObservedLocalFile::Absent;
    changed.evidence.assessment = local_recovery::LocalRecoveryAssessment::NoVisibleNames;
    assert!(matches!(
        reopened.record_local_recovery_case_once(&changed),
        Err(JournalError::RecoveryEvidenceAlreadyRecorded)
    ));
    assert_eq!(
        reopened
            .read_local_recovery_case(staged.operation_id)
            .unwrap(),
        Some(original)
    );
    reopened
        .connection
        .execute(
            "UPDATE sync_local_recovery_evidence SET evidence_digest='tampered'
             WHERE operation_id = ?1",
            [staged.operation_id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        reopened.read_local_recovery_case(staged.operation_id),
        Err(JournalError::CorruptedRecord)
    ));
    assert!(reopened
        .baseline(staged.vault_id, staged.document_id)
        .unwrap()
        .is_none());
}

#[test]
fn m1d_recovery_evidence_requires_consistent_classification_and_ticket() {
    let mut journal = SqliteSyncJournal::memory().unwrap();
    let staged = local_stage_intent(
        VaultId::new(),
        DocumentId::new(),
        "notes/recover.md",
        b"payload",
    );
    journal.begin(staged.clone()).unwrap();
    journal
        .mark_staged(staged.operation_id, b"payload")
        .unwrap();
    let evidence = m1d_test_evidence(&staged);
    assert!(matches!(
        journal.record_local_recovery_case_once(&evidence),
        Err(JournalError::CorruptedRecord)
    ));
    journal
        .record_local_publication_attempt_once(&staged)
        .unwrap();
    let mut forged = evidence.clone();
    forged.evidence.assessment = local_recovery::LocalRecoveryAssessment::NoVisibleNames;
    assert!(matches!(
        journal.record_local_recovery_case_once(&forged),
        Err(JournalError::CorruptedRecord)
    ));
    journal.record_local_recovery_case_once(&evidence).unwrap();
    assert!(journal
        .read_local_recovery_case(staged.operation_id)
        .unwrap()
        .is_some());
    assert!(journal
        .baseline(staged.vault_id, staged.document_id)
        .unwrap()
        .is_none());
}

include!("recovery.rs");
