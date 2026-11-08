use super::*;

fn intent(source: Transport) -> (ExpectedTransfer, Vec<u8>) {
    let bytes = b"receiver-owned synthetic file".to_vec();
    (
        ExpectedTransfer {
            id: Uuid::new_v4(),
            sender_key: [1; 32],
            recipient_key: [2; 32],
            source,
            bytes: bytes.len() as u64,
            blake3: *blake3::hash(&bytes).as_bytes(),
        },
        bytes,
    )
}

fn publish_fixture(ledger: &InboxLedger, v: &ExpectedTransfer, bytes: &[u8]) {
    // Simulates an *independent* no-replace file writer. The ledger itself
    // has no ability to publish bytes or choose an arbitrary destination.
    let path = ledger.object_path(v.id);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    use std::io::Write;
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}

#[test]
fn cold_restart_checks_disk_not_just_the_sqlite_row() {
    let dir = tempfile::tempdir().unwrap();
    let (v, data) = intent(Transport::P2p);
    {
        let mut inbox = InboxLedger::open(dir.path()).unwrap();
        assert_eq!(inbox.reserve(&v).unwrap(), LocalStatus::Pending);
        publish_fixture(&inbox, &v, &data);
        assert_eq!(inbox.confirm(&v).unwrap(), LocalStatus::VerifiedLocal);
        assert_eq!(inbox.count().unwrap(), 1);
    }
    let inbox = InboxLedger::open(dir.path()).unwrap();
    assert_eq!(inbox.status(v.id).unwrap(), LocalStatus::VerifiedLocal);
    assert_eq!(inbox.count().unwrap(), 1);
}

#[test]
fn same_transfer_idempotent_and_cross_provider_conflict_is_held() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = InboxLedger::open(dir.path()).unwrap();
    let (v, _data) = intent(Transport::Dropbox);
    assert_eq!(inbox.reserve(&v).unwrap(), LocalStatus::Pending);
    assert_eq!(inbox.reserve(&v).unwrap(), LocalStatus::Pending);
    assert_eq!(inbox.count().unwrap(), 1);
    let mut replaced = v.clone();
    replaced.source = Transport::S3;
    assert!(matches!(
        inbox.reserve(&replaced),
        Err(LedgerError::ConflictingIntent)
    ));
    replaced = v.clone();
    replaced.blake3 = [3; 32];
    assert!(matches!(
        inbox.reserve(&replaced),
        Err(LedgerError::ConflictingIntent)
    ));
    replaced = v.clone();
    replaced.sender_key = [4; 32];
    assert!(matches!(
        inbox.reserve(&replaced),
        Err(LedgerError::ConflictingIntent)
    ));
    assert_eq!(inbox.count().unwrap(), 1);
}

#[test]
fn provider_metadata_does_not_confirm_a_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut inbox = InboxLedger::open(dir.path()).unwrap();
    let (v, _data) = intent(Transport::S3);
    inbox.reserve(&v).unwrap();
    assert!(matches!(
        inbox.confirm(&v),
        Err(LedgerError::InvalidLocalObject)
    ));
    assert_eq!(inbox.status(v.id).unwrap(), LocalStatus::Pending);
}

#[test]
fn file_changed_or_removed_after_commit_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let (v, data) = intent(Transport::Dropbox);
    let mut inbox = InboxLedger::open(dir.path()).unwrap();
    inbox.reserve(&v).unwrap();
    publish_fixture(&inbox, &v, &data);
    assert_eq!(inbox.confirm(&v).unwrap(), LocalStatus::VerifiedLocal);
    fs::write(inbox.object_path(v.id), b"attacker replacement bytes").unwrap();
    assert_eq!(inbox.status(v.id).unwrap(), LocalStatus::NeedsReview);
    fs::remove_file(inbox.object_path(v.id)).unwrap();
    assert_eq!(inbox.status(v.id).unwrap(), LocalStatus::NeedsReview);
}

#[test]
fn file_created_before_sqlite_commit_can_be_reconciled() {
    let dir = tempfile::tempdir().unwrap();
    let (v, data) = intent(Transport::P2p);
    {
        let inbox = InboxLedger::open(dir.path()).unwrap();
        inbox.reserve(&v).unwrap();
        publish_fixture(&inbox, &v, &data);
        assert_eq!(inbox.status(v.id).unwrap(), LocalStatus::Pending);
    }
    let mut inbox = InboxLedger::open(dir.path()).unwrap();
    assert_eq!(inbox.confirm(&v).unwrap(), LocalStatus::VerifiedLocal);
    assert_eq!(inbox.count().unwrap(), 1);
}

#[test]
fn valid_file_without_reserved_intent_is_not_received() {
    let dir = tempfile::tempdir().unwrap();
    let mut inbox = InboxLedger::open(dir.path()).unwrap();
    let (v, data) = intent(Transport::S3);
    publish_fixture(&inbox, &v, &data);
    assert!(matches!(
        inbox.confirm(&v),
        Err(LedgerError::UnknownTransfer)
    ));
    assert_eq!(inbox.status(v.id).unwrap(), LocalStatus::Unknown);
}

#[test]
fn oversized_zero_or_nil_intent_never_enters_database() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = InboxLedger::open(dir.path()).unwrap();
    let (mut v, _) = intent(Transport::S3);
    v.bytes = MAX_ITEM_BYTES + 1;
    assert!(matches!(inbox.reserve(&v), Err(LedgerError::InvalidIntent)));
    v.bytes = 0;
    assert!(matches!(inbox.reserve(&v), Err(LedgerError::InvalidIntent)));
    v.bytes = 1;
    v.id = Uuid::nil();
    assert!(matches!(inbox.reserve(&v), Err(LedgerError::InvalidIntent)));
    assert_eq!(inbox.count().unwrap(), 0);
}

#[cfg(unix)]
#[test]
fn symlinked_local_object_and_inbox_root_fail_closed() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("store");
    let mut inbox = InboxLedger::open(&root).unwrap();
    let (v, data) = intent(Transport::P2p);
    inbox.reserve(&v).unwrap();
    let outside = dir.path().join("external.bin");
    fs::write(&outside, &data).unwrap();
    symlink(&outside, inbox.object_path(v.id)).unwrap();
    assert!(matches!(
        inbox.confirm(&v),
        Err(LedgerError::InvalidLocalObject)
    ));
    let linked_root = dir.path().join("linked-store");
    symlink(&root, &linked_root).unwrap();
    assert!(matches!(
        InboxLedger::open(&linked_root),
        Err(LedgerError::UnsafeRoot)
    ));
}

#[test]
fn receiver_writer_is_invoked_once_even_when_offer_replayed() {
    let dir = tempfile::tempdir().unwrap();
    let (v, data) = intent(Transport::P2p);
    let mut inbox = InboxLedger::open(dir.path()).unwrap();
    let mut writes = 0;
    let first = inbox
        .receive_with_writer(&v, |path| {
            writes += 1;
            use std::io::Write;
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)?;
            f.write_all(&data)?;
            f.sync_all()?;
            Ok(())
        })
        .unwrap();
    assert_eq!(first, LocalStatus::VerifiedLocal);
    assert_eq!(
        inbox
            .receive_with_writer(&v, |_| { panic!("replayed transfer cannot invoke writer") })
            .unwrap(),
        LocalStatus::VerifiedLocal
    );
    assert_eq!(writes, 1);
    assert_eq!(inbox.count().unwrap(), 1);
}

#[test]
fn cold_resume_adopts_exact_file_without_second_writer() {
    let dir = tempfile::tempdir().unwrap();
    let (v, data) = intent(Transport::Dropbox);
    {
        let inbox = InboxLedger::open(dir.path()).unwrap();
        inbox.reserve(&v).unwrap();
        publish_fixture(&inbox, &v, &data);
    }
    let mut inbox = InboxLedger::open(dir.path()).unwrap();
    assert_eq!(
        inbox
            .receive_with_writer(&v, |_| {
                panic!("existing file must be verified, not overwritten")
            })
            .unwrap(),
        LocalStatus::VerifiedLocal
    );
}

#[test]
fn corrupt_existing_object_cannot_be_overwritten_by_retry() {
    let dir = tempfile::tempdir().unwrap();
    let (v, _) = intent(Transport::S3);
    let mut inbox = InboxLedger::open(dir.path()).unwrap();
    inbox.reserve(&v).unwrap();
    fs::write(inbox.object_path(v.id), b"attacker modified bytes").unwrap();
    assert!(matches!(
        inbox.receive_with_writer(&v, |_| { panic!("occupied path cannot call writer") }),
        Err(LedgerError::InvalidLocalObject)
    ));
    assert_eq!(
        fs::read(inbox.object_path(v.id)).unwrap(),
        b"attacker modified bytes"
    );
    assert_eq!(inbox.status(v.id).unwrap(), LocalStatus::NeedsReview);
}

#[test]
fn writer_failure_leaves_pending_and_empty_destination_is_retryable() {
    let dir = tempfile::tempdir().unwrap();
    let (v, data) = intent(Transport::P2p);
    let mut inbox = InboxLedger::open(dir.path()).unwrap();
    assert!(matches!(
        inbox.receive_with_writer(&v, |_| { Err(LedgerError::InvalidLocalObject) }),
        Err(LedgerError::InvalidLocalObject)
    ));
    assert_eq!(inbox.status(v.id).unwrap(), LocalStatus::Pending);
    assert_eq!(
        inbox
            .receive_with_writer(&v, |path| {
                use std::io::Write;
                let mut f = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)?;
                f.write_all(&data)?;
                f.sync_all()?;
                Ok(())
            })
            .unwrap(),
        LocalStatus::VerifiedLocal
    );
}

#[test]
fn lying_writer_cannot_confirm_untrusted_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let (v, _) = intent(Transport::S3);
    let mut inbox = InboxLedger::open(dir.path()).unwrap();
    assert!(matches!(
        inbox.receive_with_writer(&v, |path| {
            fs::write(path, b"wrong bytes")?;
            Ok(())
        }),
        Err(LedgerError::InvalidLocalObject)
    ));
    assert_eq!(inbox.count().unwrap(), 1);
    assert_eq!(inbox.status(v.id).unwrap(), LocalStatus::NeedsReview);
}

#[test]
fn no_replace_race_adopts_only_the_verified_winner() {
    let dir = tempfile::tempdir().unwrap();
    let (v, data) = intent(Transport::P2p);
    let mut first = InboxLedger::open(dir.path()).unwrap();
    let mut contender = InboxLedger::open(dir.path()).unwrap();
    first.reserve(&v).unwrap();
    let mut attempts = 0;
    let outcome = contender
        .receive_with_writer(&v, |path| {
            assert_eq!(
                first.receive_with_writer(&v, |winner_path| {
                    use std::io::Write;
                    let mut file = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(winner_path)?;
                    file.write_all(&data)?;
                    file.sync_all()?;
                    Ok(())
                })?,
                LocalStatus::VerifiedLocal
            );
            attempts += 1;
            // A second no-replace publisher loses the filesystem race.
            let _ = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)?;
            Ok(())
        })
        .unwrap();
    assert_eq!(outcome, LocalStatus::VerifiedLocal);
    assert_eq!(attempts, 1);
    assert_eq!(contender.count().unwrap(), 1);
    assert_eq!(contender.status(v.id).unwrap(), LocalStatus::VerifiedLocal);
}

#[test]
fn no_replace_race_with_bad_winner_never_confirms() {
    let dir = tempfile::tempdir().unwrap();
    let (v, _) = intent(Transport::P2p);
    let mut ledger = InboxLedger::open(dir.path()).unwrap();
    assert!(matches!(
        ledger.receive_with_writer(&v, |path| {
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)?;
            file.write_all(b"wrong content")?;
            file.sync_all()?;
            // EEXIST is not enough to establish that the winner is valid.
            let _ = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)?;
            Ok(())
        }),
        Err(LedgerError::InvalidLocalObject)
    ));
    assert_eq!(ledger.status(v.id).unwrap(), LocalStatus::NeedsReview);
    assert_eq!(ledger.count().unwrap(), 1);
}

#[test]
fn export_source_requires_verified_state_and_rehashes_opened_handle() {
    let dir = tempfile::tempdir().unwrap();
    let (intent, bytes) = intent(Transport::P2p);
    let mut writer = InboxLedger::open(dir.path()).unwrap();
    writer.reserve(&intent).unwrap();
    assert!(matches!(
        writer.open_verified_file(intent.id),
        Err(LedgerError::InvalidLocalObject)
    ));
    publish_fixture(&writer, &intent, &bytes);
    assert_eq!(writer.confirm(&intent).unwrap(), LocalStatus::VerifiedLocal);
    drop(writer);
    let reader = InboxLedger::open_existing(dir.path()).unwrap().unwrap();
    let (mut source, expected) = reader.open_verified_file(intent.id).unwrap();
    assert_eq!(expected, intent);
    let mut observed = Vec::new();
    source.read_to_end(&mut observed).unwrap();
    assert_eq!(observed, bytes);
    drop(source);
    fs::write(reader.object_path(intent.id), b"tampered").unwrap();
    assert!(matches!(
        reader.open_verified_file(intent.id),
        Err(LedgerError::InvalidLocalObject)
    ));
}

#[test]
fn read_only_preview_does_not_create_a_missing_inbox() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("does-not-exist");
    assert!(InboxLedger::open_existing(&root).unwrap().is_none());
    assert!(!root.exists());
}

#[test]
fn read_only_preview_bounded_and_rechecks_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (first, first_bytes) = intent(Transport::P2p);
    let (last, last_bytes) = intent(Transport::Dropbox);
    {
        let mut writer = InboxLedger::open(root).unwrap();
        for (v, data) in [(&first, &first_bytes), (&last, &last_bytes)] {
            writer.reserve(v).unwrap();
            publish_fixture(&writer, v, data);
            assert_eq!(writer.confirm(v).unwrap(), LocalStatus::VerifiedLocal);
        }
    }
    let reader = InboxLedger::open_existing(root).unwrap().unwrap();
    assert_eq!(reader.count().unwrap(), 2);
    let recent = reader.list_recent(1).unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].0.id, last.id);
    assert_eq!(recent[0].1, LocalStatus::VerifiedLocal);
    assert!(matches!(
        reader.list_recent(0),
        Err(LedgerError::InvalidIntent)
    ));
    assert!(matches!(
        reader.list_recent(65),
        Err(LedgerError::InvalidIntent)
    ));
    fs::write(reader.object_path(last.id), b"tampered").unwrap();
    assert_eq!(
        reader.list_recent(1).unwrap()[0].1,
        LocalStatus::NeedsReview
    );
    // An SQL read-only handle cannot append a fake received item.
    assert!(reader
        .db
        .execute("UPDATE received_inbox SET state = 0", [])
        .is_err());
}

#[test]
fn separate_transports_share_one_ledger_without_cross_import() {
    let dir = tempfile::tempdir().unwrap();
    let mut inbox = InboxLedger::open(dir.path()).unwrap();
    for provider in [Transport::P2p, Transport::Dropbox, Transport::S3] {
        let (v, data) = intent(provider);
        inbox.reserve(&v).unwrap();
        publish_fixture(&inbox, &v, &data);
        assert_eq!(inbox.confirm(&v).unwrap(), LocalStatus::VerifiedLocal);
    }
    assert_eq!(inbox.count().unwrap(), 3);
    let listed = inbox.list().unwrap();
    assert_eq!(listed.len(), 3);
    assert!(listed
        .iter()
        .all(|(_expected, status)| *status == LocalStatus::VerifiedLocal));
}
