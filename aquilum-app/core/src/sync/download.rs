//! Stage 04e: verify and persist a remote object *before* any local File Gate
//! operation. This is download staging only, not document replacement.
//!
//! The existing SqliteSyncJournal stays at Pending/Staged and does not advance
//! the confirmed baseline. Native apply/rename/trash remains unavailable until
//! a host/OS-handle VaultPort and dirty-editor coordination have passed tests.

use super::{
    DagStoreError, JournalAction, JournalError, JournalPhase, OperationIntent, RemoteObject,
    RemoteStore, RemoteStoreError, SqliteDagStore, SqliteSyncJournal,
};

#[derive(Debug)]
pub enum StageDownloadError {
    InvalidIntent,
    InvalidPhase,
    InvalidObject,
    ContentMismatch,
    RemoteChangedDuringReadback,
    Journal(JournalError),
    Cache(DagStoreError),
    Remote(RemoteStoreError),
}

impl From<JournalError> for StageDownloadError {
    fn from(error: JournalError) -> Self { Self::Journal(error) }
}

impl From<DagStoreError> for StageDownloadError {
    fn from(error: DagStoreError) -> Self { Self::Cache(error) }
}

impl From<RemoteStoreError> for StageDownloadError {
    fn from(error: RemoteStoreError) -> Self { Self::Remote(error) }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedDownload {
    pub content_hash: String,
    pub byte_length: u64,
    /// The observed provider revision is an ephemeral readback fact; this
    /// staging receipt does NOT grant authority to perform a local write.
    pub observed_provider_revision: String,
    pub was_already_staged: bool,
}

/// Store verified remote bytes in the immutable local blob cache.
///
/// Every provider must explicitly implement a **bounded** object reader.
/// The legacy unbounded \`RemoteStore::read\` is NOT used here. Checks HEAD,
/// bounded bytes and HEAD again, requiring identical provider revisions and
/// verified BLAKE3. Any crash between storing bytes and journal staging is
/// safe to retry because cache objects are immutable and keyed by digest.
///
/// Most importantly: NO local note is created, replaced or deleted, and the
/// journal is never advanced past Staged by this operation.
pub fn stage_remote_download(
    journal: &mut SqliteSyncJournal,
    cache: &mut SqliteDagStore,
    remote: &dyn RemoteStore,
    intent: OperationIntent,
    max_bytes: u64,
) -> Result<StagedDownload, StageDownloadError> {
    if intent.action != JournalAction::StageDownload
        || intent.remote_guard.is_some()
        || max_bytes == 0
    {
        return Err(StageDownloadError::InvalidIntent);
    }

    let previous = journal.begin(intent.clone())?;
    match previous {
        JournalPhase::Pending | JournalPhase::Staged => {}
        _ => return Err(StageDownloadError::InvalidPhase),
    }

    let before = remote.head(&intent.remote_object_id)?;
    validate_object(&before, &intent.remote_object_id)?;
    if before.byte_length > max_bytes {
        return Err(StageDownloadError::Remote(RemoteStoreError::ObjectTooLarge));
    }

    // The provider must enforce max_bytes before producing/allocating a Vec.
    // Default RemoteStore::read_bounded fails closed.
    let bytes = remote.read_bounded(&intent.remote_object_id, max_bytes)?;
    let after = remote.head(&intent.remote_object_id)?;
    validate_object(&after, &intent.remote_object_id)?;
    if before != after || bytes.len() as u64 != after.byte_length {
        return Err(StageDownloadError::RemoteChangedDuringReadback);
    }

    let actual_hash = blake3::hash(&bytes).to_hex().to_string();
    if actual_hash != intent.expected_content_hash
        || after.content_hash.as_deref()
            .is_some_and(|claimed| claimed != actual_hash)
    {
        return Err(StageDownloadError::ContentMismatch);
    }

    // The cache verifies both the requested key and the stored bytes on read.
    // A corrupted existing record cannot be silently overwritten/accepted.
    let stored_hash = cache.store_blob(&bytes)?;
    if stored_hash != intent.expected_content_hash {
        return Err(StageDownloadError::ContentMismatch);
    }
    if cache.verified_blob(&stored_hash)? != bytes {
        return Err(StageDownloadError::ContentMismatch);
    }

    if previous == JournalPhase::Pending {
        journal.mark_staged(intent.operation_id, &bytes)?;
    }
    Ok(StagedDownload {
        content_hash: stored_hash,
        byte_length: bytes.len() as u64,
        observed_provider_revision: after.revision,
        was_already_staged: previous == JournalPhase::Staged,
    })
}

fn validate_object(object: &RemoteObject, requested_id: &str)
    -> Result<(), StageDownloadError>
{
    if object.id != requested_id || object.revision.is_empty() {
        return Err(StageDownloadError::InvalidObject);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{
        DocumentId, FakeRemoteStore, JournalPhase, OperationId, RemotePage,
        RemotePrecondition, RemoteWriteReceipt, RevisionId, SyncRelativePath, VaultId,
    };
    use std::cell::RefCell;

    fn intent(bytes: &[u8]) -> OperationIntent {
        let revision = RevisionId::new();
        OperationIntent {
            operation_id: OperationId::new(),
            vault_id: VaultId::new(),
            document_id: DocumentId::new(),
            profile_id: "fake-remote".into(),
            revision_id: revision,
            action: JournalAction::StageDownload,
            path: SyncRelativePath::parse("notes/test.md").unwrap(),
            remote_object_id: format!("immutable/{revision}"),
            remote_guard: None,
            expected_content_hash: blake3::hash(bytes).to_hex().to_string(),
            expected_baseline: None,
        }
    }

    fn stores(directory: &std::path::Path) -> (SqliteSyncJournal, SqliteDagStore) {
        (
            SqliteSyncJournal::open(&directory.join("sync-operations.sqlite")).unwrap(),
            SqliteDagStore::open(&directory.join("sync-blobs.sqlite")).unwrap(),
        )
    }

    #[test]
    fn verified_staging_is_not_an_acknowledged_local_document_write() {
        let dir = tempfile::tempdir().unwrap();
        let (mut journal, mut cache) = stores(dir.path());
        let mut remote = FakeRemoteStore::new();
        let bytes = b"remote version\n";
        let op = intent(bytes);
        remote.put(&op.remote_object_id, bytes, &RemotePrecondition::Absent).unwrap();

        let receipt = stage_remote_download(
            &mut journal, &mut cache, &remote, op.clone(), 4096,
        ).unwrap();
        assert_eq!(receipt.content_hash, op.expected_content_hash);
        assert_eq!(receipt.byte_length, bytes.len() as u64);
        assert!(!receipt.was_already_staged);
        assert_eq!(cache.verified_blob(&receipt.content_hash).unwrap(), bytes);
        assert_eq!(journal.get(op.operation_id).unwrap().unwrap().phase,
            JournalPhase::Staged);
        assert!(journal.baseline(op.vault_id, op.document_id).unwrap().is_none());

        // Same verified bytes can be checked again after process restart.
        drop(journal);
        drop(cache);
        let (mut journal, mut cache) = stores(dir.path());
        let again = stage_remote_download(
            &mut journal, &mut cache, &remote, op.clone(), 4096,
        ).unwrap();
        assert!(again.was_already_staged);
        assert_eq!(again.content_hash, receipt.content_hash);
        assert!(journal.baseline(op.vault_id, op.document_id).unwrap().is_none());
    }

    #[test]
    fn payload_meter_repeated_stage_and_one_byte_change_read_full_objects() {
        use super::super::fake::MeteredFakeRemoteStore;

        // This is an in-memory RemoteStore measurement, NOT network wire bytes.
        // Use the real stage_remote_download path (journal + durable blob cache).
        let dir = tempfile::tempdir().unwrap();
        let (mut journal, mut cache) = stores(dir.path());
        let mut remote = MeteredFakeRemoteStore::default();
        let original = vec![b'a'; 1024 * 1024];
        let original_intent = intent(&original);
        remote.put(
            &original_intent.remote_object_id,
            &original,
            &RemotePrecondition::Absent,
        ).unwrap();
        remote.reset_counts();

        let first = stage_remote_download(
            &mut journal, &mut cache, &remote, original_intent.clone(),
            original.len() as u64,
        ).unwrap();
        let cold = remote.counts();
        assert!(!first.was_already_staged);
        assert_eq!(cold.head_calls, 2);
        assert_eq!(cold.get_calls, 1);
        assert_eq!(cold.get_bytes, original.len() as u64);
        assert_eq!(cold.put_bytes, 0);
        assert!(journal.baseline(original_intent.vault_id, original_intent.document_id)
            .unwrap().is_none());

        drop(journal);
        drop(cache);
        let (mut journal, mut cache) = stores(dir.path());
        remote.reset_counts();

        let staged_again = stage_remote_download(
            &mut journal, &mut cache, &remote, original_intent.clone(),
            original.len() as u64,
        ).unwrap();
        let repeated = remote.counts();
        assert!(staged_again.was_already_staged);
        // Current conservative code reads the full object AGAIN after restart.
        // This is a measured optimization candidate, not a reason to skip
        // independent remote-version and local-file checks.
        assert_eq!(repeated.head_calls, 2);
        assert_eq!(repeated.get_calls, 1);
        assert_eq!(repeated.get_bytes, original.len() as u64);
        assert_eq!(cache.verified_blob(&original_intent.expected_content_hash)
            .unwrap(), original);

        let mut changed = original.clone();
        let middle = changed.len() / 2;
        changed[middle] ^= 1;
        let changed_intent = intent(&changed);
        remote.put(
            &changed_intent.remote_object_id,
            &changed,
            &RemotePrecondition::Absent,
        ).unwrap();
        remote.reset_counts();
        let changed_result = stage_remote_download(
            &mut journal, &mut cache, &remote, changed_intent.clone(),
            changed.len() as u64,
        ).unwrap();
        let one_byte_edit = remote.counts();
        assert!(!changed_result.was_already_staged);
        assert_eq!(one_byte_edit.head_calls, 2);
        assert_eq!(one_byte_edit.get_calls, 1);
        assert_eq!(one_byte_edit.get_bytes, changed.len() as u64);
        assert_ne!(changed_result.content_hash, staged_again.content_hash);
        assert!(journal.baseline(changed_intent.vault_id, changed_intent.document_id)
            .unwrap().is_none());

        eprintln!(
            "DATA_AUD_MOCK_STAGE original_bytes={} first_get={} repeated_get={} one_byte_edit_get={} repeated_head_calls={}",
            original.len(), cold.get_bytes, repeated.get_bytes,
            one_byte_edit.get_bytes, repeated.head_calls
        );
    }

    #[test]
    fn remote_content_mismatch_does_not_enter_the_cache_or_advance() {
        let dir = tempfile::tempdir().unwrap();
        let (mut journal, mut cache) = stores(dir.path());
        let mut remote = FakeRemoteStore::new();
        let op = intent(b"expected");
        remote.put(&op.remote_object_id, b"actually different",
            &RemotePrecondition::Absent).unwrap();

        assert!(matches!(stage_remote_download(
            &mut journal, &mut cache, &remote, op.clone(), 1024,
        ), Err(StageDownloadError::ContentMismatch)));
        assert!(journal.baseline(op.vault_id, op.document_id).unwrap().is_none());
        assert_eq!(journal.get(op.operation_id).unwrap().unwrap().phase, JournalPhase::Pending);
        assert!(matches!(cache.verified_blob(&op.expected_content_hash),
            Err(DagStoreError::MissingBlob)));
    }

    #[test]
    fn oversized_metadata_or_exact_bounded_read_prevents_allocation() {
        let dir = tempfile::tempdir().unwrap();
        let (mut journal, mut cache) = stores(dir.path());
        let mut remote = FakeRemoteStore::new();
        let op = intent(&vec![42; 100]);
        remote.put(&op.remote_object_id, &vec![42; 100],
            &RemotePrecondition::Absent).unwrap();
        assert!(matches!(stage_remote_download(
            &mut journal, &mut cache, &remote, op.clone(), 99,
        ), Err(StageDownloadError::Remote(RemoteStoreError::ObjectTooLarge))));
        assert!(matches!(remote.read_bounded(&op.remote_object_id, 99),
            Err(RemoteStoreError::ObjectTooLarge)));
        assert!(matches!(cache.verified_blob(&op.expected_content_hash),
            Err(DagStoreError::MissingBlob)));
        assert!(journal.baseline(op.vault_id, op.document_id).unwrap().is_none());
    }

    struct UnboundedOnly(FakeRemoteStore);

    impl RemoteStore for UnboundedOnly {
        fn capabilities(&self) -> super::super::ProviderCapabilities {
            self.0.capabilities()
        }
        fn list_page(&self, cursor: Option<&str>, limit: usize)
            -> Result<RemotePage, RemoteStoreError>
        {
            self.0.list_page(cursor, limit)
        }
        fn read(&self, id: &str) -> Result<Vec<u8>, RemoteStoreError> { self.0.read(id) }
        fn head(&self, id: &str) -> Result<RemoteObject, RemoteStoreError> { self.0.head(id) }
        fn put(&mut self, id: &str, data: &[u8], guard: &RemotePrecondition)
            -> Result<RemoteWriteReceipt, RemoteStoreError>
        {
            self.0.put(id, data, guard)
        }
        fn delete(&mut self, id: &str, guard: &RemotePrecondition)
            -> Result<(), RemoteStoreError>
        {
            self.0.delete(id, guard)
        }
    }

    #[test]
    fn legacy_unbounded_provider_must_opt_in_to_bounded_read() {
        let dir = tempfile::tempdir().unwrap();
        let (mut journal, mut cache) = stores(dir.path());
        let mut remote = UnboundedOnly(FakeRemoteStore::new());
        let op = intent(b"data");
        remote.put(&op.remote_object_id, b"data", &RemotePrecondition::Absent).unwrap();
        assert!(matches!(stage_remote_download(
            &mut journal, &mut cache, &remote, op.clone(), 100,
        ), Err(StageDownloadError::Remote(RemoteStoreError::PreconditionUnsupported))));
        assert!(journal.baseline(op.vault_id, op.document_id).unwrap().is_none());
    }

    struct ChangingRemote(RefCell<FakeRemoteStore>);

    impl RemoteStore for ChangingRemote {
        fn capabilities(&self) -> super::super::ProviderCapabilities {
            self.0.borrow().capabilities()
        }
        fn list_page(&self, cursor: Option<&str>, limit: usize)
            -> Result<RemotePage, RemoteStoreError>
        {
            self.0.borrow().list_page(cursor, limit)
        }
        fn read(&self, id: &str) -> Result<Vec<u8>, RemoteStoreError> {
            self.0.borrow().read(id)
        }
        fn read_bounded(&self, id: &str, max: u64)
            -> Result<Vec<u8>, RemoteStoreError>
        {
            let bytes = self.0.borrow().read_bounded(id, max)?;
            let revision = self.0.borrow().head(id)?.revision;
            self.0.borrow_mut()
                .put(id, &bytes, &RemotePrecondition::Matches(revision))?;
            Ok(bytes)
        }
        fn head(&self, id: &str) -> Result<RemoteObject, RemoteStoreError> {
            self.0.borrow().head(id)
        }
        fn put(&mut self, id: &str, data: &[u8], guard: &RemotePrecondition)
            -> Result<RemoteWriteReceipt, RemoteStoreError>
        {
            self.0.borrow_mut().put(id, data, guard)
        }
        fn delete(&mut self, id: &str, guard: &RemotePrecondition)
            -> Result<(), RemoteStoreError>
        {
            self.0.borrow_mut().delete(id, guard)
        }
    }

    #[test]
    fn object_revision_changed_during_readback_leaves_baseline_unconfirmed() {
        let dir = tempfile::tempdir().unwrap();
        let (mut journal, mut cache) = stores(dir.path());
        let mut remote = ChangingRemote(RefCell::new(FakeRemoteStore::new()));
        let op = intent(b"hello");
        remote.put(&op.remote_object_id, b"hello", &RemotePrecondition::Absent).unwrap();

        assert!(matches!(stage_remote_download(
            &mut journal, &mut cache, &remote, op.clone(), 100,
        ), Err(StageDownloadError::RemoteChangedDuringReadback)));
        assert!(matches!(cache.verified_blob(&op.expected_content_hash),
            Err(DagStoreError::MissingBlob)));
        assert!(journal.baseline(op.vault_id, op.document_id).unwrap().is_none());
        assert_eq!(journal.get(op.operation_id).unwrap().unwrap().phase, JournalPhase::Pending);
    }

    #[test]
    fn interrupted_after_blob_cache_insert_but_before_journal_mark_staged_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"saved immutable bytes";
        let op = intent(bytes);
        let (mut journal, mut cache) = stores(dir.path());
        let mut remote = FakeRemoteStore::new();
        remote.put(&op.remote_object_id, bytes, &RemotePrecondition::Absent).unwrap();
        journal.begin(op.clone()).unwrap();
        cache.store_blob(bytes).unwrap();
        drop(journal);
        drop(cache);

        let (mut journal, mut cache) = stores(dir.path());
        let receipt = stage_remote_download(
            &mut journal, &mut cache, &remote, op.clone(), 4096,
        ).unwrap();
        assert!(!receipt.was_already_staged);
        assert_eq!(journal.get(op.operation_id).unwrap().unwrap().phase,
            JournalPhase::Staged);
        assert!(journal.baseline(op.vault_id, op.document_id).unwrap().is_none());
        assert_eq!(cache.verified_blob(&receipt.content_hash).unwrap(), bytes);
    }

    #[test]
    fn empty_object_and_wrong_action_are_handled_without_mutating_documents() {
        let dir = tempfile::tempdir().unwrap();
        let (mut journal, mut cache) = stores(dir.path());
        let mut remote = FakeRemoteStore::new();
        let empty = intent(b"");
        remote.put(&empty.remote_object_id, b"", &RemotePrecondition::Absent).unwrap();
        let receipt = stage_remote_download(
            &mut journal, &mut cache, &remote, empty.clone(), 1,
        ).unwrap();
        assert_eq!(receipt.byte_length, 0);
        assert_eq!(cache.verified_blob(&receipt.content_hash).unwrap(), b"");
        let mut wrong = intent(b"");
        wrong.action = JournalAction::PublishTombstone;
        assert!(matches!(stage_remote_download(
            &mut journal, &mut cache, &remote, wrong.clone(), 1,
        ), Err(StageDownloadError::InvalidIntent)));
        assert!(journal.get(wrong.operation_id).unwrap().is_none());
    }


    #[test]
    fn reused_operation_id_cannot_replace_a_staged_blob_identity() {
        let dir = tempfile::tempdir().unwrap();
        let (mut journal, mut cache) = stores(dir.path());
        let mut remote = FakeRemoteStore::new();
        let original = b"immutable original";
        let op = intent(original);
        remote.put(&op.remote_object_id, original, &RemotePrecondition::Absent).unwrap();

        stage_remote_download(
            &mut journal, &mut cache, &remote, op.clone(), 4096,
        ).unwrap();
        let mut replay_with_other_bytes = op.clone();
        replay_with_other_bytes.expected_content_hash =
            blake3::hash(b"malicious replacement").to_hex().to_string();

        assert!(matches!(
            stage_remote_download(
                &mut journal, &mut cache, &remote, replay_with_other_bytes, 4096,
            ),
            Err(StageDownloadError::Journal(_))
        ));
        assert_eq!(
            journal.get(op.operation_id).unwrap().unwrap().phase,
            JournalPhase::Staged
        );
        assert_eq!(cache.verified_blob(&op.expected_content_hash).unwrap(), original);
        assert!(journal.baseline(op.vault_id, op.document_id).unwrap().is_none());
    }

    #[test]
    fn zero_read_bound_fails_before_persisting_an_intent() {
        let dir = tempfile::tempdir().unwrap();
        let (mut journal, mut cache) = stores(dir.path());
        let remote = FakeRemoteStore::new();
        let op = intent(b"not read");
        assert!(matches!(
            stage_remote_download(&mut journal, &mut cache, &remote, op.clone(), 0),
            Err(StageDownloadError::InvalidIntent)
        ));
        assert!(journal.get(op.operation_id).unwrap().is_none());
    }
}
