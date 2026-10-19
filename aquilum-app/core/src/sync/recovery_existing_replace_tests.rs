// Included inside recovery_dashboard::tests. Uses the existing real Core
// fixture; synthetic remote revision/physical IDs are ONLY journal test data.
// No native replacement, remote network request or user-file mutation occurs.

fn replacement_claim_for_overview(
    core: &Core,
    journal: &mut SqliteSyncJournal,
) -> OperationIntent {
    use crate::sync::protection_store::{BoundProtectedOriginal, ProtectedOriginal};
    use crate::sync::{PinnedFileIdentity, RemoteGuard};

    let local = VaultId::from_uuid(core.ui_state.list_workspaces().unwrap()[0].id);
    let remote = VaultId::new();
    assert_ne!(local, remote, "remote identity must not be guessed from workspace UUID");
    let document = DocumentId::new();
    let previous = RevisionId::new();
    let source = OperationId::new();
    let path = SyncRelativePath::parse("notes/existing.md").unwrap();
    let old = b"original must survive";
    let proposed = b"verified replacement candidate";
    journal.begin(OperationIntent {
        operation_id: source,
        vault_id: remote,
        document_id: document,
        profile_id: "remote-fixture".into(),
        revision_id: previous,
        action: JournalAction::UploadImmutable,
        path: path.clone(),
        remote_object_id: "immutable/previous".into(),
        remote_guard: Some(RemoteGuard::Absent),
        expected_content_hash: blake3::hash(old).to_hex().to_string(),
        expected_baseline: None,
    }).unwrap();
    journal.mark_staged(source, old).unwrap();
    journal.mark_applied(source).unwrap();
    journal.mark_verified(source, old, "synthetic-provider-validator").unwrap();
    journal.commit(source).unwrap();
    let intent = OperationIntent {
        operation_id: OperationId::new(),
        vault_id: remote,
        document_id: document,
        profile_id: "remote-fixture".into(),
        revision_id: RevisionId::new(),
        action: JournalAction::StageDownload,
        path: path.clone(),
        remote_object_id: "immutable/proposed".into(),
        remote_guard: None,
        expected_content_hash: blake3::hash(proposed).to_hex().to_string(),
        expected_baseline: Some(previous),
    };
    journal.begin(intent.clone()).unwrap();
    journal.mark_staged(intent.operation_id, proposed).unwrap();
    let original = BoundProtectedOriginal {
        original: ProtectedOriginal {
            operation_id: intent.operation_id,
            path,
            content_hash: blake3::hash(old).to_hex().to_string(),
            bytes: old.to_vec(),
        },
        root: PinnedFileIdentity { device: 1, file: 2 },
        file: PinnedFileIdentity { device: 1, file: 3 },
    };
    journal.record_existing_replace_attempt_once(&intent, &original).unwrap();
    intent
}

#[test]
fn existing_replace_claim_prevents_false_empty_overview_after_reopen() {
    use crate::sync::JournalPhase;
    let (_temp, core, root, appdata) = fixture();
    let original_path = root.join("keep.md");
    fs::write(&original_path, b"original must survive").unwrap();
    let db = appdata.join(RECOVERY_JOURNAL_FILENAME);
    let mut journal = SqliteSyncJournal::open(&db).unwrap();
    assert!(!journal.existing_replace_attempts_present().unwrap());
    assert_eq!(native_recovery_overview(&core, &appdata, &root),
        NativeRecoveryOverview::NoLocalPublicationClaims);
    let intent = replacement_claim_for_overview(&core, &mut journal);
    // The old overview inspected exactly these two additive-only results
    // and incorrectly returned NoLocalPublicationClaims.
    assert_eq!(journal.local_publication_count().unwrap(), 0);
    assert!(journal.local_publication_review_queue().unwrap().is_empty());
    let before = journal.read_existing_replace_attempt(intent.operation_id).unwrap();
    let baseline = journal.baseline(intent.vault_id, intent.document_id).unwrap();
    assert_eq!(native_recovery_overview(&core, &appdata, &root),
        NativeRecoveryOverview::InspectionUnavailable);
    drop(journal);
    let reopened = SqliteSyncJournal::open_existing_read_only(&db).unwrap();
    assert!(reopened.existing_replace_attempts_present().unwrap());
    assert_eq!(native_recovery_overview(&core, &appdata, &root),
        NativeRecoveryOverview::InspectionUnavailable);
    assert_eq!(reopened.read_existing_replace_attempt(intent.operation_id).unwrap(), before);
    assert_eq!(reopened.baseline(intent.vault_id, intent.document_id).unwrap(), baseline);
    assert_eq!(reopened.get(intent.operation_id).unwrap().unwrap().phase, JournalPhase::Staged);
    assert_eq!(fs::read(&original_path).unwrap(), b"original must survive");
}

#[test]
fn existing_replace_claim_blocks_partial_overview_and_stale_additive_confirmation() {
    let (_temp, core, root, appdata) = fixture();
    let (op, stage, final_file) = m1de_archived_incident(
        &core, &root, &appdata, "notes/additive.md",
    );
    let preview = native_recovery_case_preview(&core, &appdata, &root, &op.to_string()).unwrap();
    let mut journal = SqliteSyncJournal::open(&appdata.join(RECOVERY_JOURNAL_FILENAME)).unwrap();
    replacement_claim_for_overview(&core, &mut journal);
    assert_eq!(journal.local_publication_count().unwrap(), 1);
    assert_eq!(native_recovery_overview(&core, &appdata, &root),
        NativeRecoveryOverview::InspectionUnavailable);
    assert_eq!(native_record_recovery_decision(
        &core, &appdata, &root, &preview,
        NativeRecoveryDecision::KeepBlocked, "old additive-only snapshot",
    ), Err(NativeRecoveryReviewError::NotReady));
    assert!(journal.recovery_review_history(op).unwrap().is_empty());
    assert_eq!(fs::read(stage).unwrap(), b"fixture from remote");
    assert_eq!(fs::read(final_file).unwrap(), b"other user version");
}

#[test]
fn corrupt_or_missing_replacement_table_cannot_make_overview_clean() {
    let (_temp, core, root, appdata) = fixture();
    let db = appdata.join(RECOVERY_JOURNAL_FILENAME);
    let mut journal = SqliteSyncJournal::open(&db).unwrap();
    let intent = replacement_claim_for_overview(&core, &mut journal);
    drop(journal);
    let direct = rusqlite::Connection::open(&db).unwrap();
    direct.execute(
        "UPDATE sync_existing_replace_attempts SET evidence_json=?1, evidence_digest='damaged'",
        [b"malformed evidence".as_slice()],
    ).unwrap();
    let read_only = SqliteSyncJournal::open_existing_read_only(&db).unwrap();
    assert!(read_only.read_existing_replace_attempt(intent.operation_id).is_err());
    assert!(read_only.existing_replace_attempts_present().unwrap());
    assert_eq!(native_recovery_overview(&core, &appdata, &root),
        NativeRecoveryOverview::InspectionUnavailable);
    drop(read_only);
    direct.execute_batch("DROP TABLE sync_existing_replace_attempts;").unwrap();
    assert_eq!(native_recovery_overview(&core, &appdata, &root),
        NativeRecoveryOverview::InspectionUnavailable);
    let count: i64 = direct.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE name='sync_existing_replace_attempts'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 0, "inspection must not repair a missing table");
    let version: i64 = direct.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap();
    assert_eq!(version, 7, "no read-side migration");
}
