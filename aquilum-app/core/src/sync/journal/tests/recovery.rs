#[test]
fn m1d_schema_two_migrates_without_losing_original_publication_claims() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sync.sqlite");
    let staged = local_stage_intent(
        VaultId::new(),
        DocumentId::new(),
        "notes/old-claim.md",
        b"payload",
    );
    {
        let mut j = SqliteSyncJournal::open(&path).unwrap();
        j.begin(staged.clone()).unwrap();
        j.mark_staged(staged.operation_id, b"payload").unwrap();
        j.record_local_publication_attempt_once(&staged).unwrap();
        j.connection
            .execute_batch(
                "DROP TABLE sync_local_native_publication_observations;
                 DROP TABLE sync_local_recovery_reviews;
                 DROP TABLE sync_local_recovery_copies;
                 DROP TABLE sync_local_recovery_evidence;
                 DROP TABLE IF EXISTS sync_existing_replace_receipt_candidates;
                 DROP TABLE IF EXISTS sync_existing_replace_attempts;
                  PRAGMA user_version = 2;",
            )
            .unwrap();
    }
    let upgraded = SqliteSyncJournal::open(&path).unwrap();
    assert_eq!(
        upgraded
            .connection
            .query_row::<i64, _, _>("PRAGMA user_version", [], |row| row.get(0))
            .unwrap(),
        SCHEMA_VERSION
    );
    assert_eq!(upgraded.local_publication_review_queue().unwrap().len(), 1);
    assert!(matches!(
        upgraded.record_local_publication_attempt_once(&staged),
        Err(JournalError::PublicationAttemptAlreadyRecorded)
    ));
    let evidence = m1d_test_evidence(&staged);
    upgraded.record_local_recovery_case_once(&evidence).unwrap();
    assert_eq!(
        upgraded
            .read_local_recovery_case(staged.operation_id)
            .unwrap(),
        Some(evidence)
    );
    assert!(upgraded
        .baseline(staged.vault_id, staged.document_id)
        .unwrap()
        .is_none());
}

#[test]
fn m1db_copy_is_write_once_and_tampering_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("sync.sqlite");
    let staged = local_stage_intent(
        VaultId::new(),
        DocumentId::new(),
        "notes/stage.md",
        b"payload",
    );
    let historical = m1d_test_evidence(&staged);
    {
        let mut j = SqliteSyncJournal::open(&db).unwrap();
        j.begin(staged.clone()).unwrap();
        j.mark_staged(staged.operation_id, b"payload").unwrap();
        j.record_local_publication_attempt_once(&staged).unwrap();
        j.record_local_recovery_case_once(&historical).unwrap();
        let archived = j
            .record_local_recovery_copy_once(staged.operation_id, Some(b"payload"), None)
            .unwrap();
        assert_eq!(archived.stage_bytes.as_deref(), Some(&b"payload"[..]));
        assert_eq!(archived.final_bytes, None);
        assert!(matches!(
            j.record_local_recovery_copy_once(staged.operation_id, Some(b"payload"), None),
            Err(JournalError::RecoveryCopyAlreadyRecorded)
        ));
        assert!(matches!(
            j.record_local_recovery_copy_once(staged.operation_id, Some(b"changed"), None),
            Err(JournalError::CorruptedRecord)
        ));
    }
    let j = SqliteSyncJournal::open(&db).unwrap();
    assert_eq!(
        j.read_local_recovery_copy(staged.operation_id)
            .unwrap()
            .unwrap()
            .stage_bytes
            .as_deref(),
        Some(&b"payload"[..]),
    );
    j.connection
        .execute(
            "UPDATE sync_local_recovery_copies SET stage_bytes = ?1 WHERE operation_id = ?2",
            params![b"corrupt".as_slice(), staged.operation_id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        j.read_local_recovery_copy(staged.operation_id),
        Err(JournalError::CorruptedRecord)
    ));
    assert!(j
        .baseline(staged.vault_id, staged.document_id)
        .unwrap()
        .is_none());
}

#[test]
fn m1db_archive_requires_matching_historical_observation() {
    let mut j = SqliteSyncJournal::memory().unwrap();
    let staged = local_stage_intent(
        VaultId::new(),
        DocumentId::new(),
        "notes/stage.md",
        b"payload",
    );
    j.begin(staged.clone()).unwrap();
    j.mark_staged(staged.operation_id, b"payload").unwrap();
    j.record_local_publication_attempt_once(&staged).unwrap();
    assert!(matches!(
        j.record_local_recovery_copy_once(staged.operation_id, Some(b"payload"), None),
        Err(JournalError::CorruptedRecord)
    ));
    j.record_local_recovery_case_once(&m1d_test_evidence(&staged))
        .unwrap();
    for (stage, final_name) in [
        (None, None),
        (Some(&b"payload"[..]), Some(&b"unexpected"[..])),
        (Some(&b"wrong"[..]), None),
    ] {
        assert!(matches!(
            j.record_local_recovery_copy_once(staged.operation_id, stage, final_name),
            Err(JournalError::CorruptedRecord)
        ));
    }
    assert!(j
        .read_local_recovery_copy(staged.operation_id)
        .unwrap()
        .is_none());
}

#[test]
fn m1db_schema_three_migrates_without_dropping_incident_or_claim() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("sync.sqlite");
    let staged = local_stage_intent(
        VaultId::new(),
        DocumentId::new(),
        "notes/stage.md",
        b"payload",
    );
    let case = m1d_test_evidence(&staged);
    {
        let mut journal = SqliteSyncJournal::open(&db).unwrap();
        journal.begin(staged.clone()).unwrap();
        journal
            .mark_staged(staged.operation_id, b"payload")
            .unwrap();
        journal
            .record_local_publication_attempt_once(&staged)
            .unwrap();
        journal.record_local_recovery_case_once(&case).unwrap();
        journal
            .connection
            .execute_batch(
                "DROP TABLE sync_local_native_publication_observations;
                 DROP TABLE sync_local_recovery_reviews;
                 DROP TABLE sync_local_recovery_copies;
                 DROP TABLE IF EXISTS sync_existing_replace_receipt_candidates;
                 DROP TABLE IF EXISTS sync_existing_replace_attempts;
                  PRAGMA user_version = 3;",
            )
            .unwrap();
    }
    let journal = SqliteSyncJournal::open(&db).unwrap();
    let version: i64 = journal
        .connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    assert_eq!(
        journal
            .read_local_recovery_case(staged.operation_id)
            .unwrap(),
        Some(case)
    );
    assert_eq!(journal.local_publication_review_queue().unwrap().len(), 1);
    assert!(matches!(
        journal.record_local_publication_attempt_once(&staged),
        Err(JournalError::PublicationAttemptAlreadyRecorded)
    ));
    assert!(journal
        .read_local_recovery_copy(staged.operation_id)
        .unwrap()
        .is_none());
    journal
        .record_local_recovery_copy_once(staged.operation_id, Some(b"payload"), None)
        .unwrap();
    assert!(journal
        .read_local_recovery_copy(staged.operation_id)
        .unwrap()
        .is_some());
    assert!(journal
        .baseline(staged.vault_id, staged.document_id)
        .unwrap()
        .is_none());
}

fn m1dc_fixture(db: &Path) -> (SqliteSyncJournal, OperationIntent) {
    let mut journal = SqliteSyncJournal::open(db).unwrap();
    let intent = local_stage_intent(
        VaultId::new(),
        DocumentId::new(),
        "notes/crash-review.md",
        b"payload",
    );
    journal.begin(intent.clone()).unwrap();
    journal
        .mark_staged(intent.operation_id, b"payload")
        .unwrap();
    journal
        .record_local_publication_attempt_once(&intent)
        .unwrap();
    journal
        .record_local_recovery_case_once(&m1d_test_evidence(&intent))
        .unwrap();
    journal
        .record_local_recovery_copy_once(intent.operation_id, Some(b"payload"), None)
        .unwrap();
    (journal, intent)
}

#[test]
fn m1dc_schema_four_upgrades_preserving_archive_and_review_eligibility() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("sync.sqlite");
    let (journal, intent) = m1dc_fixture(&db);
    journal
        .connection
        .execute_batch(
            "DROP TABLE sync_local_native_publication_observations;
                 DROP TABLE sync_local_recovery_reviews;
             DROP TABLE IF EXISTS sync_existing_replace_receipt_candidates;
                 DROP TABLE IF EXISTS sync_existing_replace_attempts;
                  PRAGMA user_version = 4;",
        )
        .unwrap();
    drop(journal);
    let mut upgraded = SqliteSyncJournal::open(&db).unwrap();
    let version: i64 = upgraded
        .connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    assert_eq!(
        upgraded
            .read_local_recovery_case(intent.operation_id)
            .unwrap()
            .unwrap()
            .operation_id,
        intent.operation_id
    );
    assert_eq!(
        upgraded
            .read_local_recovery_copy(intent.operation_id)
            .unwrap()
            .unwrap()
            .stage_bytes,
        Some(b"payload".to_vec())
    );
    assert_eq!(upgraded.local_publication_review_queue().unwrap().len(), 1);
    let screen = upgraded
        .recovery_review_snapshot(intent.operation_id)
        .unwrap();
    assert_eq!(screen.review_count, 0);
    let event = upgraded
        .append_recovery_review(
            intent.operation_id,
            intent.vault_id,
            &intent.path,
            &screen.archive_fingerprint,
            0,
            "genesis",
            RecoveryReviewDisposition::KeepBlocked,
            "Hold forensic evidence",
        )
        .unwrap();
    assert_eq!(event.sequence, 1);
    assert!(upgraded
        .baseline(intent.vault_id, intent.document_id)
        .unwrap()
        .is_none());
    drop(upgraded);
    let opened = SqliteSyncJournal::open(&db).unwrap();
    assert_eq!(
        opened.recovery_review_history(intent.operation_id).unwrap(),
        vec![event]
    );
    assert!(opened
        .baseline(intent.vault_id, intent.document_id)
        .unwrap()
        .is_none());
    assert!(matches!(
        opened.get(intent.operation_id).unwrap().unwrap().phase,
        JournalPhase::Staged
    ));
}

#[test]
fn m1dc_two_connections_refuse_stale_revision_and_preserve_chain() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("sync.sqlite");
    let (first, intent) = m1dc_fixture(&db);
    drop(first);
    let mut one = SqliteSyncJournal::open(&db).unwrap();
    let mut two = SqliteSyncJournal::open(&db).unwrap();
    let screen = one.recovery_review_snapshot(intent.operation_id).unwrap();
    let first_event = one
        .append_recovery_review(
            intent.operation_id,
            intent.vault_id,
            &intent.path,
            &screen.archive_fingerprint,
            screen.review_count,
            &screen.latest_event_digest,
            RecoveryReviewDisposition::KeepBlocked,
            "Investigation pending",
        )
        .unwrap();
    assert!(matches!(
        two.append_recovery_review(
            intent.operation_id,
            intent.vault_id,
            &intent.path,
            &screen.archive_fingerprint,
            screen.review_count,
            &screen.latest_event_digest,
            RecoveryReviewDisposition::EscalateManualReview,
            "Race with previous decision",
        ),
        Err(JournalError::StaleRecoveryReview)
    ));
    let refreshed = two.recovery_review_snapshot(intent.operation_id).unwrap();
    let second_event = two
        .append_recovery_review(
            intent.operation_id,
            intent.vault_id,
            &intent.path,
            &refreshed.archive_fingerprint,
            refreshed.review_count,
            &refreshed.latest_event_digest,
            RecoveryReviewDisposition::EscalateManualReview,
            "Needs independent operator approval",
        )
        .unwrap();
    assert_eq!(second_event.sequence, 2);
    assert_eq!(
        second_event.previous_event_digest,
        refreshed.latest_event_digest
    );
    assert_eq!(
        two.recovery_review_history(intent.operation_id).unwrap(),
        vec![first_event.clone(), second_event]
    );
    assert_eq!(
        one.recovery_review_history(intent.operation_id)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(one.local_publication_review_queue().unwrap().len(), 1);
    assert!(one
        .baseline(intent.vault_id, intent.document_id)
        .unwrap()
        .is_none());
}

#[test]
fn m1dc_corrupted_review_row_or_broken_sequence_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("sync.sqlite");
    let (mut j, intent) = m1dc_fixture(&db);
    let screen = j.recovery_review_snapshot(intent.operation_id).unwrap();
    j.append_recovery_review(
        intent.operation_id,
        intent.vault_id,
        &intent.path,
        &screen.archive_fingerprint,
        0,
        "genesis",
        RecoveryReviewDisposition::KeepBlocked,
        "Retain content",
    )
    .unwrap();
    j.connection
        .execute(
            "UPDATE sync_local_recovery_reviews
             SET review_digest='tampered'
             WHERE operation_id=?1 AND sequence=1",
            [intent.operation_id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        j.recovery_review_history(intent.operation_id),
        Err(JournalError::CorruptedRecord)
    ));
    j.connection
        .execute(
            "DELETE FROM sync_local_recovery_reviews WHERE operation_id=?1",
            [intent.operation_id.to_string()],
        )
        .unwrap();
    let screen = j.recovery_review_snapshot(intent.operation_id).unwrap();
    j.append_recovery_review(
        intent.operation_id,
        intent.vault_id,
        &intent.path,
        &screen.archive_fingerprint,
        0,
        "genesis",
        RecoveryReviewDisposition::KeepBlocked,
        "Retain content",
    )
    .unwrap();
    j.connection
        .execute(
            "UPDATE sync_local_recovery_reviews
             SET sequence=3
             WHERE operation_id=?1 AND sequence=1",
            [intent.operation_id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        j.recovery_review_history(intent.operation_id),
        Err(JournalError::CorruptedRecord)
    ));
    assert!(j
        .baseline(intent.vault_id, intent.document_id)
        .unwrap()
        .is_none());
}

#[test]
fn m1dc_missing_or_damaged_archive_is_not_eligible_for_review() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("sync.sqlite");
    let (j, intent) = m1dc_fixture(&db);
    j.connection
        .execute(
            "DELETE FROM sync_local_recovery_copies WHERE operation_id=?1",
            [intent.operation_id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        j.recovery_review_snapshot(intent.operation_id),
        Err(JournalError::MissingRecoveryArchive)
    ));
    assert!(j
        .baseline(intent.vault_id, intent.document_id)
        .unwrap()
        .is_none());
}

#[test]
fn m1dd_existing_journal_inspector_is_strictly_sqlite_read_only() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("existing.sqlite3");
    let mut original = SqliteSyncJournal::open(&path).unwrap();
    let pending = local_stage_intent(
        VaultId::new(),
        DocumentId::new(),
        "notes/safe.md",
        b"byte content",
    );
    original.begin(pending.clone()).unwrap();
    original
        .mark_staged(pending.operation_id, b"byte content")
        .unwrap();
    original
        .record_local_publication_attempt_once(&pending)
        .unwrap();
    drop(original);
    let readonly = SqliteSyncJournal::open_existing_read_only(&path).unwrap();
    assert_eq!(readonly.local_publication_count().unwrap(), 1);
    assert_eq!(readonly.local_publication_review_queue().unwrap().len(), 1);
    assert!(readonly
        .connection
        .execute(
            "UPDATE sync_operations SET phase = 4 WHERE operation_id = ?1",
            [pending.operation_id.to_string()],
        )
        .is_err());
    assert!(readonly
        .connection
        .execute("DELETE FROM sync_local_publication_attempts", [],)
        .is_err());
    assert!(readonly
        .baseline(pending.vault_id, pending.document_id)
        .unwrap()
        .is_none());
}

#[test]
fn m1e4_schema_five_upgrades_without_losing_existing_crash_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sync.sqlite");
    let (journal, staged) = m1dc_fixture(&path);
    assert!(journal
        .read_native_local_publication_observation(staged.operation_id)
        .unwrap()
        .is_none());
    journal
        .connection
        .execute_batch(
            "DROP TABLE sync_local_native_publication_observations;
             DROP TABLE IF EXISTS sync_existing_replace_receipt_candidates;
                 DROP TABLE IF EXISTS sync_existing_replace_attempts;
                  PRAGMA user_version = 5;",
        )
        .unwrap();
    drop(journal);
    // A read-only Settings inspection cannot silently migrate the DB.
    assert!(matches!(
        SqliteSyncJournal::open_existing_read_only(&path),
        Err(JournalError::UnsupportedSchema(5))
    ));
    let migrated = SqliteSyncJournal::open(&path).unwrap();
    let version: i64 = migrated
        .connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    assert_eq!(
        migrated
            .read_local_recovery_case(staged.operation_id)
            .unwrap()
            .unwrap()
            .operation_id,
        staged.operation_id
    );
    assert_eq!(
        migrated
            .read_local_recovery_copy(staged.operation_id)
            .unwrap()
            .unwrap()
            .stage_bytes,
        Some(b"payload".to_vec())
    );
    assert_eq!(migrated.local_publication_review_queue().unwrap().len(), 1);
    assert!(
        migrated
            .read_native_local_publication_observation(staged.operation_id)
            .unwrap()
            .is_none(),
        "schema migration cannot invent an OS receipt"
    );
    assert!(migrated
        .baseline(staged.vault_id, staged.document_id)
        .unwrap()
        .is_none());
}

#[cfg(unix)]
#[test]
fn m1e4_write_once_unix_audit_corruption_never_releases_baseline() {
    let mut journal = SqliteSyncJournal::memory().unwrap();
    let staged = local_stage_intent(
        VaultId::new(),
        DocumentId::new(),
        "notes/unix-publish.md",
        b"durably synced",
    );
    journal.begin(staged.clone()).unwrap();
    journal
        .mark_staged(staged.operation_id, b"durably synced")
        .unwrap();
    journal
        .record_local_publication_attempt_once(&staged)
        .unwrap();
    let obs = NativeLocalPublicationObservation {
        operation_id: staged.operation_id,
        vault_id: staged.vault_id,
        path: staged.path.clone(),
        expected_content_hash: staged.expected_content_hash.clone(),
        parent: PinnedFileIdentity {
            device: 10,
            file: 11,
        },
        final_file: PinnedFileIdentity {
            device: 10,
            file: 12,
        },
        byte_length: b"durably synced".len() as u64,
    };
    assert!(journal
        .read_native_local_publication_observation(staged.operation_id)
        .unwrap()
        .is_none());
    journal
        .record_native_local_publication_observation_once(&obs)
        .unwrap();
    assert_eq!(
        journal
            .read_native_local_publication_observation(staged.operation_id)
            .unwrap(),
        Some(obs.clone())
    );
    assert!(matches!(
        journal.record_native_local_publication_observation_once(&obs),
        Err(JournalError::NativePublicationObservationAlreadyRecorded)
    ));
    assert!(matches!(
        journal.mark_applied(staged.operation_id),
        Err(JournalError::LocalDownloadNeedsNativeReceipt)
    ));
    assert!(matches!(
        journal.commit(staged.operation_id),
        Err(JournalError::LocalDownloadNeedsNativeReceipt)
    ));
    assert!(journal
        .baseline(staged.vault_id, staged.document_id)
        .unwrap()
        .is_none());

    journal
        .connection
        .execute(
            "UPDATE sync_local_native_publication_observations
             SET observation_json = ?1 WHERE operation_id = ?2",
            params![b"tampered".as_slice(), staged.operation_id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        journal.read_native_local_publication_observation(staged.operation_id),
        Err(JournalError::CorruptedRecord)
    ));
    assert_eq!(
        journal.get(staged.operation_id).unwrap().unwrap().phase,
        JournalPhase::Staged
    );
    assert!(journal
        .baseline(staged.vault_id, staged.document_id)
        .unwrap()
        .is_none());
}

#[test]
fn newer_schema_must_never_be_overwritten() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA user_version = 99;").unwrap();
    assert!(matches!(
        SqliteSyncJournal::prepare(conn),
        Err(JournalError::UnsupportedSchema(99))
    ));
}
