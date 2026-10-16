//! Adversarial, deterministic stage 04 matrix: 12 remote lifecycle/fault
//! conditions x 6 payload classes = 72 independently executed cases.
//!
//! These are NOT a substitute for native OS-handle security, remote integration
//! or power-off testing; they exercise the journal/provider protocol boundary.
//! Every failure case asserts that the confirmed baseline never advances
//! based on an unavailable, ambiguous, altered or mismatched observation.

use super::{
    upload_immutable_verified, DocumentId, FakeRemoteStore, JournalAction, JournalPhase,
    OperationId, OperationIntent, RemoteGuard, RemoteObject, RemotePage, RemotePrecondition,
    RemoteStore, RemoteStoreError, RemoteWriteReceipt, RevisionId, SqliteSyncJournal,
    SyncRelativePath, UploadOutcome, VaultId, VerifiedUploadError,
};
use std::cell::{Cell, RefCell};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scenario {
    Normal,
    LostAck,
    PreviouslyStaged,
    PreviouslyWrittenLostAck,
    ExistingIdenticalObject,
    ExistingConflictingObject,
    HeadUnavailable,
    PutUnavailable,
    ReadUnavailable,
    CorruptedRead,
    ChangedRevisionDuringRead,
    CorruptedHeadHash,
}

impl Scenario {
    const ALL: [Self; 12] = [
        Self::Normal,
        Self::LostAck,
        Self::PreviouslyStaged,
        Self::PreviouslyWrittenLostAck,
        Self::ExistingIdenticalObject,
        Self::ExistingConflictingObject,
        Self::HeadUnavailable,
        Self::PutUnavailable,
        Self::ReadUnavailable,
        Self::CorruptedRead,
        Self::ChangedRevisionDuringRead,
        Self::CorruptedHeadHash,
    ];

    fn fails_once(self) -> bool {
        matches!(
            self,
            Self::HeadUnavailable
                | Self::PutUnavailable
                | Self::ReadUnavailable
                | Self::CorruptedRead
                | Self::ChangedRevisionDuringRead
                | Self::CorruptedHeadHash
        )
    }
}

struct AdversarialRemote {
    inner: RefCell<FakeRemoteStore>,
    scenario: Scenario,
    fault_active: Cell<bool>,
    put_calls: Cell<usize>,
    head_calls: Cell<usize>,
    read_calls: Cell<usize>,
}

impl AdversarialRemote {
    fn new(scenario: Scenario) -> Self {
        Self {
            inner: RefCell::new(FakeRemoteStore::new()),
            scenario,
            fault_active: Cell::new(true),
            put_calls: Cell::new(0),
            head_calls: Cell::new(0),
            read_calls: Cell::new(0),
        }
    }

    fn disable_fault(&self) {
        self.fault_active.set(false);
    }

    fn bytes(&self, key: &str) -> Result<Vec<u8>, RemoteStoreError> {
        self.inner.borrow().read(key)
    }

    fn count(&self) -> usize {
        self.inner.borrow().object_count()
    }

    fn put_calls(&self) -> usize {
        self.put_calls.get()
    }

    fn prepopulate(&self, key: &str, bytes: &[u8]) -> String {
        self.inner.borrow_mut()
            .put(key, bytes, &RemotePrecondition::Absent)
            .unwrap()
            .revision
    }
}

impl RemoteStore for AdversarialRemote {
    fn capabilities(&self) -> super::ProviderCapabilities {
        self.inner.borrow().capabilities()
    }

    fn list_page(&self, continuation: Option<&str>, page_limit: usize)
        -> Result<RemotePage, RemoteStoreError>
    {
        self.inner.borrow().list_page(continuation, page_limit)
    }

    fn head(&self, key: &str) -> Result<RemoteObject, RemoteStoreError> {
        self.head_calls.set(self.head_calls.get() + 1);
        if self.fault_active.get() && self.scenario == Scenario::HeadUnavailable {
            return Err(RemoteStoreError::Unavailable);
        }
        let mut object = self.inner.borrow().head(key)?;
        if self.fault_active.get() && self.scenario == Scenario::CorruptedHeadHash {
            object.content_hash = Some("untrusted-provider-metadata".into());
        }
        Ok(object)
    }

    fn read(&self, key: &str) -> Result<Vec<u8>, RemoteStoreError> {
        self.read_calls.set(self.read_calls.get() + 1);
        if self.fault_active.get() && self.scenario == Scenario::ReadUnavailable {
            return Err(RemoteStoreError::Unavailable);
        }
        let mut bytes = self.inner.borrow().read(key)?;
        if self.fault_active.get() && self.scenario == Scenario::CorruptedRead {
            bytes.push(0xff);
        }
        if self.fault_active.get() && self.scenario == Scenario::ChangedRevisionDuringRead {
            // Same bytes, different opaque revision, between the coordinator's
            // two head calls. The coordinator must refuse this unstable read.
            let version = self.inner.borrow().head(key)?.revision;
            self.inner.borrow_mut()
                .put(key, &bytes, &RemotePrecondition::Matches(version))?;
        }
        Ok(bytes)
    }

    fn put(&mut self, key: &str, bytes: &[u8], condition: &RemotePrecondition)
        -> Result<RemoteWriteReceipt, RemoteStoreError>
    {
        self.put_calls.set(self.put_calls.get() + 1);
        if self.fault_active.get() && self.scenario == Scenario::PutUnavailable {
            return Err(RemoteStoreError::Unavailable);
        }
        let mut inner = self.inner.borrow_mut();
        if self.fault_active.get() && self.scenario == Scenario::LostAck {
            inner.lose_next_write_ack();
        }
        inner.put(key, bytes, condition)
    }

    fn delete(&mut self, key: &str, condition: &RemotePrecondition)
        -> Result<(), RemoteStoreError>
    {
        self.inner.borrow_mut().delete(key, condition)
    }
}

fn payload(n: usize) -> Vec<u8> {
    (0..n).map(|i| ((i * 197 + 31) % 256) as u8).collect()
}

fn intent(data: &[u8]) -> OperationIntent {
    let revision = RevisionId::new();
    OperationIntent {
        operation_id: OperationId::new(),
        vault_id: VaultId::new(),
        document_id: DocumentId::new(),
        profile_id: "fault-matrix/provider-v1".into(),
        revision_id: revision,
        action: JournalAction::UploadImmutable,
        path: SyncRelativePath::parse("Notes/Example.md").unwrap(),
        remote_object_id: format!("immutable/{revision}"),
        remote_guard: Some(RemoteGuard::Absent),
        expected_content_hash: blake3::hash(data).to_hex().to_string(),
        expected_baseline: None,
    }
}

fn run_case(size: usize, scenario: Scenario) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.sqlite");
    let bytes = payload(size);
    let intent = intent(&bytes);
    let mut journal = SqliteSyncJournal::open(&path).unwrap();
    let mut remote = AdversarialRemote::new(scenario);
    let key = intent.remote_object_id.clone();

    match scenario {
        Scenario::PreviouslyStaged | Scenario::PreviouslyWrittenLostAck => {
            journal.begin(intent.clone()).unwrap();
            journal.mark_staged(intent.operation_id, &bytes).unwrap();
        }
        _ => {}
    }
    match scenario {
        Scenario::PreviouslyWrittenLostAck | Scenario::ExistingIdenticalObject => {
            remote.prepopulate(&key, &bytes);
        }
        Scenario::ExistingConflictingObject => {
            remote.prepopulate(&key, b"foreign bytes must not be overwritten");
        }
        _ => {}
    }
    // Reopening the app-data database simulates a crash/restart, with no
    // synthetic delete or forced reupload after a possibly lost acknowledgement.
    if matches!(scenario, Scenario::PreviouslyStaged | Scenario::PreviouslyWrittenLostAck) {
        drop(journal);
        journal = SqliteSyncJournal::open(&path).unwrap();
    }

    let result = upload_immutable_verified(&mut journal, &mut remote, intent.clone(), &bytes);
    let initial_put_calls = remote.put_calls();

    match scenario {
        Scenario::ExistingConflictingObject => {
            assert!(matches!(result, Err(VerifiedUploadError::ContentMismatch)),
                "size={size} scenario={scenario:?} result={result:?}");
            assert!(journal.baseline(intent.vault_id, intent.document_id).unwrap().is_none());
            assert_eq!(remote.count(), 1);
            assert_eq!(remote.put_calls(), 0, "must not overwrite foreign bytes");
            assert_eq!(remote.bytes(&key).unwrap(), b"foreign bytes must not be overwritten");
            return;
        }
        case if case.fails_once() => {
            match case {
                Scenario::HeadUnavailable | Scenario::PutUnavailable
                    | Scenario::ReadUnavailable => {
                    assert!(matches!(result, Err(VerifiedUploadError::Remote(
                        RemoteStoreError::Unavailable
                    ))), "size={size} scenario={case:?} result={result:?}");
                }
                Scenario::CorruptedRead | Scenario::CorruptedHeadHash => {
                    assert!(matches!(result, Err(VerifiedUploadError::ContentMismatch)),
                        "size={size} scenario={case:?} result={result:?}");
                }
                Scenario::ChangedRevisionDuringRead => {
                    assert!(matches!(result,
                        Err(VerifiedUploadError::RemoteChangedDuringReadback)),
                        "size={size} scenario={case:?} result={result:?}");
                }
                _ => unreachable!(),
            }
            assert!(journal.baseline(intent.vault_id, intent.document_id).unwrap().is_none());
            assert_eq!(journal.recovery_queue().unwrap().len(), 1);
            remote.disable_fault();
            assert_eq!(
                upload_immutable_verified(&mut journal, &mut remote, intent.clone(), &bytes)
                    .unwrap(),
                UploadOutcome::Committed,
                "size={size} scenario={case:?}"
            );
            if case == Scenario::ReadUnavailable
                || case == Scenario::CorruptedRead
                || case == Scenario::CorruptedHeadHash
                || case == Scenario::ChangedRevisionDuringRead
            {
                assert_eq!(remote.put_calls(), initial_put_calls,
                    "readback retry must not submit another upload");
            }
        }
        _ => assert!(matches!(result, Ok(UploadOutcome::Committed)),
            "size={size} scenario={scenario:?} result={result:?}"),
    }

    let baseline = journal.baseline(intent.vault_id, intent.document_id)
        .unwrap().expect("only verified content may become a baseline");
    assert_eq!(baseline.revision_id, intent.revision_id);
    assert_eq!(baseline.content_hash, blake3::hash(&bytes).to_hex().to_string());
    assert_eq!(journal.get(intent.operation_id).unwrap().unwrap().phase,
        JournalPhase::Committed);
    assert!(journal.recovery_queue().unwrap().is_empty());
    assert_eq!(remote.count(), 1);
    assert_eq!(remote.bytes(&key).unwrap(), bytes);

    let previous_put_calls = remote.put_calls();
    assert_eq!(
        upload_immutable_verified(&mut journal, &mut remote, intent.clone(), &bytes).unwrap(),
        UploadOutcome::AlreadyCommitted,
        "size={size} scenario={scenario:?}"
    );
    assert_eq!(remote.put_calls(), previous_put_calls,
        "idempotent retries cannot write again");

    // A fresh process sees the same verified baseline and no recovery work.
    drop(journal);
    let reopened = SqliteSyncJournal::open(&path).unwrap();
    assert_eq!(reopened.baseline(intent.vault_id, intent.document_id)
        .unwrap().unwrap().revision_id, intent.revision_id);
    assert!(reopened.recovery_queue().unwrap().is_empty());
}

#[test]
fn twelve_fault_conditions_across_six_payload_sizes_are_fail_closed() {
    const SIZES: [usize; 6] = [0, 1, 17, 256, 4096, 65536];
    let mut cases = 0;
    for size in SIZES {
        for scenario in Scenario::ALL {
            run_case(size, scenario);
            cases += 1;
        }
    }
    assert_eq!(cases, 72);
}

#[test]
fn stale_baseline_or_second_inflight_write_never_uploads() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = SqliteSyncJournal::open(&dir.path().join("journal.sqlite")).unwrap();
    let mut remote = AdversarialRemote::new(Scenario::Normal);
    let a = intent(b"first");
    upload_immutable_verified(&mut journal, &mut remote, a.clone(), b"first").unwrap();

    let mut stale = intent(b"stale");
    stale.vault_id = a.vault_id;
    stale.document_id = a.document_id;
    assert!(matches!(upload_immutable_verified(&mut journal, &mut remote, stale,
        b"stale"), Err(VerifiedUploadError::Journal(
            super::JournalError::BaselineChanged
        ))));
    assert_eq!(remote.count(), 1);

    let b = intent(b"second");
    journal.begin(b.clone()).unwrap();
    let mut competing = intent(b"competing");
    competing.vault_id = b.vault_id;
    competing.document_id = b.document_id;
    assert!(matches!(upload_immutable_verified(&mut journal, &mut remote,
        competing, b"competing"), Err(VerifiedUploadError::Journal(
            super::JournalError::BusyDocument
        ))));
    assert_eq!(remote.count(), 1);
}

#[test]
fn refusing_head_probes_on_uncertain_commit_preserves_a_retryable_journal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.sqlite");
    let mut journal = SqliteSyncJournal::open(&path).unwrap();
    let mut remote = AdversarialRemote::new(Scenario::HeadUnavailable);
    let op = intent(b"will not be uploaded");
    let result = upload_immutable_verified(&mut journal, &mut remote, op.clone(),
        b"will not be uploaded");
    assert!(matches!(result, Err(VerifiedUploadError::Remote(
        RemoteStoreError::Unavailable
    ))));
    assert_eq!(remote.count(), 0);
    assert!(journal.baseline(op.vault_id, op.document_id).unwrap().is_none());
    drop(journal);
    let reopened = SqliteSyncJournal::open(&path).unwrap();
    assert_eq!(reopened.recovery_queue().unwrap().len(), 1);
    assert_eq!(reopened.recovery_queue().unwrap()[0].action,
        super::RecoveryAction::InspectBeforeRetry);
}
