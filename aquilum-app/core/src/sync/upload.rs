//! Stage 04c: conservative immutable-object upload coordinator.
//!
//! The orchestrator can be tested against FakeRemoteStore with an independent
//! durable SqliteSyncJournal. It has NO native File Gate access and NO provider
//! registration. The future provider adapter must enforce its declared atomic
//! create-if-absent capability, and must supply real destination readback.

use super::{
    JournalAction, JournalError, JournalPhase, OperationIntent, RemoteGuard, RemoteObject,
    RemotePrecondition, RemoteStore, RemoteStoreError, SqliteSyncJournal,
};

#[derive(Debug)]
pub enum VerifiedUploadError {
    Journal(JournalError),
    Remote(RemoteStoreError),
    InvalidIntent,
    WeakProvider,
    ContentMismatch,
    RemoteConflict,
    LostConfirmation,
    RemoteChangedDuringReadback,
}

impl From<JournalError> for VerifiedUploadError {
    fn from(error: JournalError) -> Self { Self::Journal(error) }
}

impl From<RemoteStoreError> for VerifiedUploadError {
    fn from(error: RemoteStoreError) -> Self { Self::Remote(error) }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadOutcome {
    Committed,
    AlreadyCommitted,
}

/// Safe v1 subset: upload a new immutable object to a provider that supports
/// real create-if-absent, not a simulated best-effort check-and-set.
///
/// Nothing is blindly retried after an uncertain write: head+readback must
/// prove the exact new bytes and an unchanged provider revision before any
/// journal state or baseline advances. If readback fails, return to caller
/// with the journal still available for another inspection.
/// This never overwrites or deletes an existing remote object.
pub fn upload_immutable_verified(
    journal: &mut SqliteSyncJournal,
    remote: &mut dyn RemoteStore,
    intent: OperationIntent,
    bytes: &[u8],
) -> Result<UploadOutcome, VerifiedUploadError> {
    if intent.action != JournalAction::UploadImmutable
        || !matches!(intent.remote_guard.as_ref(), Some(RemoteGuard::Absent))
    {
        return Err(VerifiedUploadError::InvalidIntent);
    }
    if !remote.capabilities().conditional_create {
        return Err(VerifiedUploadError::WeakProvider);
    }
    let hash = blake3::hash(bytes).to_hex().to_string();
    if hash != intent.expected_content_hash {
        return Err(VerifiedUploadError::ContentMismatch);
    }

    let id = intent.operation_id;
    let current = journal.begin(intent.clone())?;
    if current == JournalPhase::Committed {
        return Ok(UploadOutcome::AlreadyCommitted);
    }
    if current == JournalPhase::Pending {
        journal.mark_staged(id, bytes)?;
    }

    // Always inspect the remote before deciding whether to (re)try a staged
    // mutation, because a failed HTTP response can hide a successful write.
    // A journal marked Applied/Verified must NOT attempt another upload.
    let mut new_receipt = None;
    match remote.head(&intent.remote_object_id) {
        Ok(_) => {}
        Err(RemoteStoreError::NotFound) if current == JournalPhase::Pending
            || current == JournalPhase::Staged =>
        {
            match remote.put(
                &intent.remote_object_id, bytes, &RemotePrecondition::Absent,
            ) {
                Ok(receipt) => { new_receipt = Some(receipt.revision); }
                Err(RemoteStoreError::CommitUncertain) => {
                    // Possible success; perform verified readback below.
                }
                Err(RemoteStoreError::PreconditionFailed) => {
                    // Could be a concurrent writer, not our lost ACK. Never
                    // reinterpret a conflicting object as safe to overwrite.
                    return Err(VerifiedUploadError::RemoteConflict);
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(RemoteStoreError::NotFound) => return Err(VerifiedUploadError::LostConfirmation),
        Err(error) => return Err(error.into()),
    }

    let (object, observed) = stable_readback(remote, &intent.remote_object_id)?;
    if let Some(expected_provider_revision) = new_receipt {
        if expected_provider_revision != object.revision {
            return Err(VerifiedUploadError::RemoteChangedDuringReadback);
        }
    }
    if blake3::hash(&observed).to_hex().as_str() != intent.expected_content_hash {
        return Err(VerifiedUploadError::ContentMismatch);
    }
    if object.content_hash.as_deref().is_some_and(|known| {
        known != intent.expected_content_hash
    }) {
        return Err(VerifiedUploadError::ContentMismatch);
    }

    let record = journal.get(id)?.ok_or(VerifiedUploadError::LostConfirmation)?;
    match record.phase {
        JournalPhase::Staged => { journal.mark_applied(id)?; }
        JournalPhase::Applied => {}
        JournalPhase::Verified => {
            if record.provider_revision.as_deref() != Some(object.revision.as_str()) {
                return Err(VerifiedUploadError::RemoteChangedDuringReadback);
            }
        }
        _ => return Err(VerifiedUploadError::LostConfirmation),
    }
    if record.phase != JournalPhase::Verified {
        journal.mark_verified(id, &observed, &object.revision)?;
    }
    journal.commit(id)?;
    Ok(UploadOutcome::Committed)
}

/// Re-read the remote validator around the actual bytes; a changed ETag or
/// provider revision is not proof that the same immutable object was read.
/// Provider-native revision identity is *not* a BLAKE3 content digest.
fn stable_readback(
    remote: &dyn RemoteStore, object_id: &str,
) -> Result<(RemoteObject, Vec<u8>), VerifiedUploadError> {
    let before = remote.head(object_id)?;
    if before.id != object_id || before.revision.is_empty() {
        return Err(VerifiedUploadError::RemoteChangedDuringReadback);
    }
    let observed = remote.read(object_id)?;
    let after = remote.head(object_id)?;
    if after != before || observed.len() as u64 != after.byte_length {
        return Err(VerifiedUploadError::RemoteChangedDuringReadback);
    }
    Ok((after, observed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{
        DocumentId, FakeRemoteStore, FakeVaultPort, OperationId, RemotePage,
        RemoteWriteReceipt, RevisionId, SyncRelativePath, VaultId, VaultPort,
    };
    use tempfile::TempDir;

    fn fixture() -> (TempDir, SqliteSyncJournal, FakeRemoteStore, OperationIntent) {
        let dir = tempfile::tempdir().unwrap();
        let journal = SqliteSyncJournal::open(&dir.path().join("ops.sqlite")).unwrap();
        let revision = RevisionId::new();
        let intent = OperationIntent {
            operation_id: OperationId::new(),
            vault_id: VaultId::new(),
            document_id: DocumentId::new(),
            profile_id: "fake/s3-profile-v1".into(),
            revision_id: revision,
            action: JournalAction::UploadImmutable,
            path: SyncRelativePath::parse("Notes/work.md").unwrap(),
            remote_object_id: format!("immutable/{revision}"),
            remote_guard: Some(RemoteGuard::Absent),
            expected_content_hash: blake3::hash(b"hello world").to_hex().to_string(),
            expected_baseline: None,
        };
        (dir, journal, FakeRemoteStore::new(), intent)
    }

    #[test]
    fn verified_immutable_upload_advances_one_baseline_and_is_idempotent() {
        let (_dir, mut journal, mut remote, intent) = fixture();
        let outcome = upload_immutable_verified(
            &mut journal, &mut remote, intent.clone(), b"hello world",
        ).unwrap();
        assert_eq!(outcome, UploadOutcome::Committed);
        let baseline = journal.baseline(intent.vault_id, intent.document_id)
            .unwrap().unwrap();
        assert_eq!(baseline.revision_id, intent.revision_id);
        assert_eq!(baseline.operation_id, intent.operation_id);
        assert_eq!(remote.object_count(), 1);
        let first_revision = remote.head(&intent.remote_object_id).unwrap().revision;
        assert_eq!(upload_immutable_verified(&mut journal, &mut remote, intent.clone(),
            b"hello world").unwrap(), UploadOutcome::AlreadyCommitted);
        assert_eq!(remote.head(&intent.remote_object_id).unwrap().revision, first_revision);
        assert!(journal.recovery_queue().unwrap().is_empty());
    }

    #[test]
    fn payload_meter_full_upload_requires_one_verified_readback() {
        use crate::sync::fake::MeteredFakeRemoteStore;

        // Count mock RemoteStore payload bytes, NOT QUIC/IP bytes.
        // This protects the readback invariant; it is not an optimization.
        let (_dir, mut journal, _unused, mut intent) = fixture();
        let bytes = vec![b'x'; 1024 * 1024];
        intent.expected_content_hash = blake3::hash(&bytes).to_hex().to_string();
        let mut remote = MeteredFakeRemoteStore::default();

        let result = upload_immutable_verified(
            &mut journal, &mut remote, intent.clone(), &bytes,
        ).unwrap();
        assert_eq!(result, UploadOutcome::Committed);
        let initial = remote.counts();
        assert_eq!(initial.put_calls, 1);
        assert_eq!(initial.put_bytes, bytes.len() as u64);
        assert_eq!(initial.get_calls, 1);
        assert_eq!(initial.get_bytes, bytes.len() as u64);
        assert_eq!(initial.head_calls, 3);
        assert!(journal.baseline(intent.vault_id, intent.document_id)
            .unwrap().is_some());

        // A confirmed repeat has no remote PUT, GET or HEAD.
        remote.reset_counts();
        assert_eq!(
            upload_immutable_verified(
                &mut journal, &mut remote, intent.clone(), &bytes,
            ).unwrap(),
            UploadOutcome::AlreadyCommitted
        );
        let repeat = remote.counts();
        assert_eq!(repeat.put_bytes, 0);
        assert_eq!(repeat.get_bytes, 0);
        assert_eq!(repeat.head_calls, 0);

        eprintln!(
            "DATA_AUD_MOCK_UPLOAD payload_bytes={} initial_put={} readback_get={} committed_repeat_get={}",
            bytes.len(), initial.put_bytes, initial.get_bytes,
            repeat.get_bytes
        );
    }

    #[test]
    fn lost_ack_is_resolved_by_remote_readback_not_by_second_put() {
        let (_dir, mut journal, mut remote, intent) = fixture();
        remote.lose_next_write_ack();
        assert_eq!(upload_immutable_verified(&mut journal, &mut remote, intent.clone(),
            b"hello world").unwrap(), UploadOutcome::Committed);
        assert_eq!(remote.object_count(), 1);
        assert!(journal.baseline(intent.vault_id, intent.document_id).unwrap().is_some());
    }

    #[test]
    fn crash_after_remote_write_before_local_ack_replays_without_duplicating() {
        let (dir, mut journal, mut remote, intent) = fixture();
        journal.begin(intent.clone()).unwrap();
        journal.mark_staged(intent.operation_id, b"hello world").unwrap();
        let original = remote.put(&intent.remote_object_id, b"hello world",
            &RemotePrecondition::Absent).unwrap().revision;
        drop(journal);
        let mut reloaded = SqliteSyncJournal::open(&dir.path().join("ops.sqlite")).unwrap();
        assert_eq!(upload_immutable_verified(&mut reloaded, &mut remote, intent.clone(),
            b"hello world").unwrap(), UploadOutcome::Committed);
        assert_eq!(remote.object_count(), 1);
        assert_eq!(remote.head(&intent.remote_object_id).unwrap().revision, original);
        assert!(reloaded.recovery_queue().unwrap().is_empty());
    }

    #[test]
    fn existing_different_object_never_gets_overwritten_or_accepted() {
        let (_dir, mut journal, mut remote, intent) = fixture();
        remote.put(&intent.remote_object_id, b"an unrelated object",
            &RemotePrecondition::Absent).unwrap();
        let result = upload_immutable_verified(&mut journal, &mut remote,
            intent.clone(), b"hello world");
        assert!(matches!(result, Err(VerifiedUploadError::ContentMismatch)));
        assert!(journal.baseline(intent.vault_id, intent.document_id).unwrap().is_none());
        assert_eq!(remote.read(&intent.remote_object_id).unwrap(), b"an unrelated object");
        assert_eq!(journal.recovery_queue().unwrap().len(), 1);
    }

    #[test]
    fn corrupted_source_bytes_are_rejected_before_even_creating_an_operation() {
        let (_dir, mut journal, mut remote, intent) = fixture();
        let result = upload_immutable_verified(
            &mut journal, &mut remote, intent.clone(), b"altered",
        );
        assert!(matches!(result, Err(VerifiedUploadError::ContentMismatch)));
        assert!(journal.get(intent.operation_id).unwrap().is_none());
        assert_eq!(remote.object_count(), 0);
    }

    struct WeakProvider(FakeRemoteStore);
    impl RemoteStore for WeakProvider {
        fn capabilities(&self) -> super::super::ProviderCapabilities {
            let mut cap = self.0.capabilities();
            cap.conditional_create = false;
            cap
        }
        fn list_page(&self, c: Option<&str>, n: usize)
            -> Result<RemotePage, RemoteStoreError> { self.0.list_page(c, n) }
        fn read(&self, id: &str) -> Result<Vec<u8>, RemoteStoreError> { self.0.read(id) }
        fn head(&self, id: &str) -> Result<RemoteObject, RemoteStoreError> { self.0.head(id) }
        fn put(&mut self, id: &str, bytes: &[u8], guard: &RemotePrecondition)
            -> Result<RemoteWriteReceipt, RemoteStoreError> { self.0.put(id, bytes, guard) }
        fn delete(&mut self, id: &str, guard: &RemotePrecondition)
            -> Result<(), RemoteStoreError> { self.0.delete(id, guard) }
    }

    #[test]
    fn weak_provider_and_wrong_action_do_not_create_journal_operations() {
        let (_dir, mut journal, fake, intent) = fixture();
        let mut weak = WeakProvider(fake);
        assert!(matches!(upload_immutable_verified(&mut journal, &mut weak,
            intent.clone(), b"hello world"), Err(VerifiedUploadError::WeakProvider)));
        assert!(journal.get(intent.operation_id).unwrap().is_none());
        let mut invalid = intent.clone();
        invalid.action = JournalAction::PublishTombstone;
        assert!(matches!(upload_immutable_verified(&mut journal, &mut weak,
            invalid, b"hello world"), Err(VerifiedUploadError::InvalidIntent)));
        assert!(journal.get(intent.operation_id).unwrap().is_none());
    }

    #[test]
    fn local_fake_snapshot_and_verified_remote_exchange_preserve_byte_hash() {
        let (_dir, mut journal, mut remote, mut intent) = fixture();
        let mut local = FakeVaultPort::new();
        let path = SyncRelativePath::parse("Notes/work.md").unwrap();
        local.create_if_absent(&path, b"hello world", OperationId::new()).unwrap();
        let bytes = local.read_snapshot(&path).unwrap();
        intent.expected_content_hash = bytes.content_hash.clone();
        assert_eq!(upload_immutable_verified(&mut journal, &mut remote, intent.clone(),
            &bytes.bytes).unwrap(), UploadOutcome::Committed);
        assert_eq!(remote.read(&intent.remote_object_id).unwrap(), bytes.bytes);
    }
}
