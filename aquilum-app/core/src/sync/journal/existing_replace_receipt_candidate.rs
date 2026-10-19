//! P0g-3d-04: immutable SQLite V8 local *receipt candidate*.
//!
//! NOT a native durable receipt. It records a point-in-time correlation of
//! the V7 one-shot fence, protected original, published file identity and an
//! immutable HistoryService CreateNew witness. Distinct SQLite databases,
//! the workspace and history directories are NOT one atomic transaction.
//! There is no transition to StageDownload Applied/Verified/Committed here.
//! All cold-recovered candidates remain HOLD / NeedsReview.

use super::*;
use crate::history::service::VerifiedHistoryVersion;
use crate::sync::protection_store::BoundProtectedOriginal;
use crate::sync::pinned_read::IdentifiedRead;

/// Never export a constructor accepting an untyped history-version ID.
/// The only production builder accepts HistoryService's non-Clone, sealed
/// VerifiedHistoryVersion and checks its exact bytes/size against the V7 target.
#[must_use = "forensic receipt candidates never grant automatic S3 apply"]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub(crate) struct InertExistingReplaceReceiptCandidate {
    operation_id: OperationId,
    vault_id: VaultId,
    document_id: DocumentId,
    path: SyncRelativePath,
    baseline_revision: RevisionId,
    baseline_operation: OperationId,
    intent_digest: String,
    preimage_hash: String,
    target_hash: String,
    original_root: PinnedFileIdentity,
    original_file: PinnedFileIdentity,
    published_file: PinnedFileIdentity,
    original_length: u64,
    target_length: u64,
    history_version_id: String,
    history_hash: String,
    history_length: u64,
}

#[allow(dead_code)]
impl InertExistingReplaceReceiptCandidate {
    /// A witness from an actual held-custody File Gate publication may be
    /// consumed here; building the candidate does not grant a new publisher.
    /// A later cold verifier MUST re-read the original BLOB, native destination,
    /// history version, registered root and V7 baseline before any decision.
    pub(crate) fn from_strict_history(
        claim: &ReadOnlyExistingReplaceAttempt,
        protected: &BoundProtectedOriginal,
        history: &VerifiedHistoryVersion,
        observed_target: &IdentifiedRead,
    ) -> JournalResult<Self> {
        // Native file identity and bytes MUST come from the same OS-pinned
        // descriptor read, not two independent path lookups supplied by host.
        let published_file = observed_target.identity
            .ok_or(JournalError::InvalidTransition)?;
        let observed_target_bytes = &observed_target.snapshot.bytes;
        if observed_target.snapshot.content_hash != claim.target_hash
            || observed_target_bytes.len() > 32 * 1024 * 1024
            || protected.original.operation_id != claim.operation_id
            || protected.original.path != claim.path
            || protected.original.content_hash != claim.preimage_hash
            || protected.original.bytes.len() as u64 != claim.original_length
            || blake3::hash(&protected.original.bytes).to_hex().as_str()
                != claim.preimage_hash
            || protected.root != claim.root
            || protected.file != claim.original_file
            || blake3::hash(observed_target_bytes).to_hex().as_str()
                != claim.target_hash
            || history.content_hash() != claim.target_hash
            || history.content_length() != observed_target_bytes.len()
        {
            return Err(JournalError::InvalidTransition);
        }
        let candidate = Self {
            operation_id: claim.operation_id,
            vault_id: claim.vault_id,
            document_id: claim.document_id,
            path: claim.path.clone(),
            baseline_revision: claim.baseline_revision,
            baseline_operation: claim.baseline_operation,
            intent_digest: claim.intent_digest.clone(),
            preimage_hash: claim.preimage_hash.clone(),
            target_hash: claim.target_hash.clone(),
            original_root: claim.root,
            original_file: claim.original_file,
            published_file,
            original_length: claim.original_length,
            target_length: observed_target_bytes.len() as u64,
            history_version_id: history.version_id().to_owned(),
            history_hash: history.content_hash().to_owned(),
            history_length: history.content_length() as u64,
        };
        candidate.check_fields()?;
        Ok(candidate)
    }

    pub(crate) fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    /// Check every field, not only the outer SQLite payload digest.
    fn check_fields(&self) -> JournalResult<()> {
        let good_hash = |v: &str| v.len() == 64
            && v.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c));
        if !good_hash(&self.preimage_hash)
            || !good_hash(&self.target_hash)
            || self.preimage_hash == self.target_hash
            || !good_hash(&self.intent_digest)
            || !good_hash(&self.history_hash)
            || self.history_hash != self.target_hash
            || self.history_version_id.is_empty()
            || self.history_version_id.len() > 255
            || self.history_version_id.chars().any(|c|
                c.is_control() || c == '/' || c == '\\')
            || self.target_length > 32 * 1024 * 1024
            || self.original_length > 32 * 1024 * 1024
            || self.history_length != self.target_length
            || self.original_root.device == 0 || self.original_root.file == 0
            || self.original_file.device == 0 || self.original_file.file == 0
            || self.published_file.device == 0 || self.published_file.file == 0
            || self.original_root.device != self.original_file.device
            || self.original_root.device != self.published_file.device
            || self.published_file == self.original_file
        {
            return Err(JournalError::InvalidTransition);
        }
        Ok(())
    }

    fn check_claim_and_baseline(
        &self,
        claim: &ReadOnlyExistingReplaceAttempt,
        record: &JournalRecord,
        baseline: &ConfirmedBaseline,
    ) -> JournalResult<()> {
        self.check_fields()?;
        if self.operation_id != claim.operation_id
            || self.vault_id != claim.vault_id
            || self.document_id != claim.document_id
            || self.path != claim.path
            || self.baseline_revision != claim.baseline_revision
            || self.baseline_operation != claim.baseline_operation
            || self.intent_digest != claim.intent_digest
            || self.preimage_hash != claim.preimage_hash
            || self.target_hash != claim.target_hash
            || self.original_root != claim.root
            || self.original_file != claim.original_file
            || self.original_length != claim.original_length
            || record.phase != JournalPhase::Staged
            || record.intent.action != JournalAction::StageDownload
            || record.intent.operation_id != self.operation_id
            || record.intent.vault_id != self.vault_id
            || record.intent.document_id != self.document_id
            || record.intent.path != self.path
            || record.intent.expected_baseline != Some(self.baseline_revision)
            || record.intent.expected_content_hash != self.target_hash
            || record.staged_hash.as_deref() != Some(self.target_hash.as_str())
            || baseline.vault_id != self.vault_id
            || baseline.document_id != self.document_id
            || baseline.revision_id != self.baseline_revision
            || baseline.operation_id != self.baseline_operation
            || baseline.content_hash != self.preimage_hash
            || baseline.profile_id != record.intent.profile_id
        {
            return Err(JournalError::InvalidTransition);
        }
        Ok(())
    }
}

impl SqliteSyncJournal {
    /// Write-once durable *metadata* within the SAME SQLite WAL/FULL domain
    /// as V7. Primary/history/protected-original durability remains unproven.
    /// The caller cannot use this function to advance sync phase/baseline.
    #[allow(dead_code)]
    pub(crate) fn record_existing_replace_receipt_candidate_once(
        &mut self,
        candidate: &InertExistingReplaceReceiptCandidate,
    ) -> JournalResult<()> {
        let claim = self.read_existing_replace_attempt(candidate.operation_id)?
            .ok_or(JournalError::InvalidTransition)?;
        let transaction = self.connection.transaction_with_behavior(
            TransactionBehavior::Immediate,
        )?;
        let record = read_operation(&transaction, candidate.operation_id)?
            .ok_or(JournalError::MissingOperation)?;
        let baseline = load_baseline(
            &transaction, candidate.vault_id, candidate.document_id,
        )?.ok_or(JournalError::BaselineChanged)?;
        candidate.check_claim_and_baseline(&claim, &record, &baseline)?;
        let original_payload = serde_json::to_vec(&claim)?;
        let original_digest = blake3::hash(&original_payload).to_hex().to_string();
        let bytes = serde_json::to_vec(candidate)?;
        let digest = blake3::hash(&bytes).to_hex().to_string();
        // The rechecked V7 evidence hash, journal stage and baseline MUST all
        // agree within this single SQLite transaction. No implicit overwrite.
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO sync_existing_replace_receipt_candidates
                (operation_id,candidate_json,candidate_digest)
             SELECT ?1,?2,?3
               FROM sync_existing_replace_attempts AS claim
               JOIN sync_operations AS op ON op.operation_id=claim.operation_id
               JOIN sync_baselines AS base
                 ON base.vault_id=op.vault_id
                AND base.document_id=op.document_id
              WHERE claim.operation_id=?1 AND claim.evidence_digest=?4
                AND op.phase=1 AND op.staged_hash=?5
                AND base.revision_id=?6 AND base.operation_id=?7
                AND base.content_hash=?8",
            params![
                candidate.operation_id.to_string(),bytes,digest,
                original_digest,&candidate.target_hash,
                candidate.baseline_revision.to_string(),
                candidate.baseline_operation.to_string(),
                &candidate.preimage_hash,
            ],
        )?;
        if inserted != 1 {
            return Err(JournalError::ExistingReplaceReceiptCandidateAlreadyRecorded);
        }
        transaction.commit()?;
        // Commit can succeed although readback fails: never retry blindly.
        if self.read_existing_replace_receipt_candidate(candidate.operation_id)?
            .as_ref() != Some(candidate)
        {
            return Err(JournalError::CorruptedRecord);
        }
        Ok(())
    }

    /// Corruption-checked read for forensic recovery ONLY.
    /// A persisted candidate cannot authorize local S3 apply, an ACK,
    /// deletion of the original, or ConfirmedBaseline promotion.
    #[allow(dead_code)]
    pub(crate) fn read_existing_replace_receipt_candidate(
        &self,
        id: OperationId,
    ) -> JournalResult<Option<InertExistingReplaceReceiptCandidate>> {
        let row: Option<(Vec<u8>,String)> = self.connection.query_row(
            "SELECT candidate_json,candidate_digest
               FROM sync_existing_replace_receipt_candidates
              WHERE operation_id=?1",
            [id.to_string()],
            |r| Ok((r.get(0)?,r.get(1)?)),
        ).optional()?;
        let Some((bytes,digest))=row else {return Ok(None)};
        if digest != blake3::hash(&bytes).to_hex().as_str() {
            return Err(JournalError::CorruptedRecord);
        }
        let candidate:InertExistingReplaceReceiptCandidate =
            serde_json::from_slice(&bytes).map_err(|_|JournalError::CorruptedRecord)?;
        let claim = self.read_existing_replace_attempt(id)?
            .ok_or(JournalError::CorruptedRecord)?;
        let record = self.get(id)?.ok_or(JournalError::CorruptedRecord)?;
        let baseline = self.baseline(candidate.vault_id,candidate.document_id)?
            .ok_or(JournalError::CorruptedRecord)?;
        if candidate.operation_id != id
            || candidate.check_claim_and_baseline(&claim,&record,&baseline).is_err()
        {
            return Err(JournalError::CorruptedRecord);
        }
        Ok(Some(candidate))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::protection_store::{ProtectedOriginal,BoundProtectedOriginal};
    use crate::sync::RemoteGuard;

    fn fixture() -> (
        tempfile::TempDir, SqliteSyncJournal, OperationIntent,
        BoundProtectedOriginal, InertExistingReplaceReceiptCandidate,
    ) {
        let dir=tempfile::tempdir().unwrap();
        let mut journal=SqliteSyncJournal::open(&dir.path().join("sync.sqlite")).unwrap();
        let vault=VaultId::new();
        let document=DocumentId::new();
        let path=SyncRelativePath::parse("notes/receipt.md").unwrap();
        let old=b"preserved source";
        let target=b"external immutable bytes";
        let rev=RevisionId::new();
        let old_operation=OperationId::new();
        let seed=OperationIntent {
            operation_id:old_operation,vault_id:vault,document_id:document,
            profile_id:"approved".into(),revision_id:rev,
            action:JournalAction::UploadImmutable,path:path.clone(),
            remote_object_id:"seed/source".into(),remote_guard:Some(RemoteGuard::Absent),
            expected_content_hash:blake3::hash(old).to_hex().to_string(),
            expected_baseline:None,
        };
        journal.begin(seed).unwrap();
        journal.mark_staged(old_operation,old).unwrap();
        journal.mark_applied(old_operation).unwrap();
        journal.mark_verified(old_operation,old,"remote-old").unwrap();
        journal.commit(old_operation).unwrap();
        let operation=OperationId::new();
        let intent=OperationIntent {
            operation_id:operation,vault_id:vault,document_id:document,
            profile_id:"approved".into(),revision_id:RevisionId::new(),
            action:JournalAction::StageDownload,path:path.clone(),
            remote_object_id:"stage/new".into(),remote_guard:None,
            expected_content_hash:blake3::hash(target).to_hex().to_string(),
            expected_baseline:Some(rev),
        };
        journal.begin(intent.clone()).unwrap();
        journal.mark_staged(operation,target).unwrap();
        let protected=BoundProtectedOriginal {
            original: ProtectedOriginal{
                operation_id:operation,path:path.clone(),
                content_hash:blake3::hash(old).to_hex().to_string(),
                bytes:old.to_vec(),
            },
            root:PinnedFileIdentity{device:11,file:24},
            file:PinnedFileIdentity{device:11,file:25},
        };
        let digest=blake3::hash(&serde_json::to_vec(&intent).unwrap())
            .to_hex().to_string();
        let candidate=InertExistingReplaceReceiptCandidate {
            operation_id:operation,vault_id:vault,document_id:document,
            path,baseline_revision:rev,baseline_operation:old_operation,
            intent_digest:digest,
            preimage_hash:protected.original.content_hash.clone(),
            target_hash:intent.expected_content_hash.clone(),
            original_root:protected.root,original_file:protected.file,
            published_file:PinnedFileIdentity{device:11,file:26},
            original_length:old.len() as u64,target_length:target.len() as u64,
            history_version_id:"1234567890000_remote_external.md".into(),
            history_hash:intent.expected_content_hash.clone(),
            history_length:target.len() as u64,
        };
        (dir,journal,intent,protected,candidate)
    }

    #[test]
    fn v8_candidate_needs_v7_claim_and_never_grants_apply() {
        let (dir,mut journal,intent,protected,candidate)=fixture();
        assert!(journal.record_existing_replace_receipt_candidate_once(&candidate).is_err());
        assert!(journal.read_existing_replace_receipt_candidate(intent.operation_id)
            .unwrap().is_none());
        journal.record_existing_replace_attempt_once(&intent,&protected).unwrap();
        journal.record_existing_replace_receipt_candidate_once(&candidate).unwrap();
        let mut second_writer=SqliteSyncJournal::open(
            &dir.path().join("sync.sqlite"),
        ).unwrap();
        assert!(matches!(
            second_writer.record_existing_replace_receipt_candidate_once(&candidate),
            Err(JournalError::ExistingReplaceReceiptCandidateAlreadyRecorded)
        ), "a second SQLite connection must not duplicate V8 evidence");
        drop(second_writer);
        assert!(matches!(
            journal.record_existing_replace_receipt_candidate_once(&candidate),
            Err(JournalError::ExistingReplaceReceiptCandidateAlreadyRecorded)
        ));
        assert!(matches!(
            journal.mark_applied(intent.operation_id),
            Err(JournalError::LocalDownloadNeedsNativeReceipt)
        ));
        drop(journal);
        let cold=SqliteSyncJournal::open_existing_read_only(
            &dir.path().join("sync.sqlite"),
        ).unwrap();
        assert_eq!(cold.read_existing_replace_receipt_candidate(intent.operation_id)
            .unwrap(),Some(candidate));
        let baseline=cold.baseline(intent.vault_id,intent.document_id).unwrap().unwrap();
        assert_eq!(baseline.revision_id,intent.expected_baseline.unwrap());
        assert_eq!(cold.get(intent.operation_id).unwrap().unwrap().phase,
            JournalPhase::Staged);
    }

    #[test]
    fn foreign_inode_wrong_history_and_wrong_baseline_are_rejected() {
        let (_dir,mut journal,intent,protected,candidate)=fixture();
        journal.record_existing_replace_attempt_once(&intent,&protected).unwrap();
        for altered in [
            InertExistingReplaceReceiptCandidate {
                published_file:protected.file,..candidate.clone()
            },
            InertExistingReplaceReceiptCandidate {
                history_hash:protected.original.content_hash.clone(),..candidate.clone()
            },
            InertExistingReplaceReceiptCandidate {
                history_version_id:"../escape".into(),..candidate.clone()
            },
            InertExistingReplaceReceiptCandidate {
                baseline_revision:RevisionId::new(),..candidate.clone()
            },
        ] {
            assert!(journal.record_existing_replace_receipt_candidate_once(&altered).is_err());
            assert!(journal.read_existing_replace_receipt_candidate(intent.operation_id)
                .unwrap().is_none());
        }
        journal.record_existing_replace_receipt_candidate_once(&candidate).unwrap();
    }

    #[test]
    fn tampering_with_digest_or_evidence_fails_closed_after_cold_open() {
        let (dir,mut journal,intent,protected,candidate)=fixture();
        journal.record_existing_replace_attempt_once(&intent,&protected).unwrap();
        journal.record_existing_replace_receipt_candidate_once(&candidate).unwrap();
        drop(journal);
        let c=Connection::open(dir.path().join("sync.sqlite")).unwrap();
        c.execute(
            "UPDATE sync_existing_replace_receipt_candidates
                 SET candidate_digest='corrupt' WHERE operation_id=?1",
            [intent.operation_id.to_string()],
        ).unwrap();
        drop(c);
        let cold=SqliteSyncJournal::open_existing_read_only(
            &dir.path().join("sync.sqlite"),
        ).unwrap();
        assert!(matches!(
            cold.read_existing_replace_receipt_candidate(intent.operation_id),
            Err(JournalError::CorruptedRecord)
        ));
    }

    #[test]
    fn changed_baseline_invalidates_existing_candidate_on_restart() {
        let (dir,mut journal,intent,protected,candidate)=fixture();
        journal.record_existing_replace_attempt_once(&intent,&protected).unwrap();
        journal.record_existing_replace_receipt_candidate_once(&candidate).unwrap();
        drop(journal);
        let c=Connection::open(dir.path().join("sync.sqlite")).unwrap();
        c.execute(
            "UPDATE sync_baselines SET revision_id=?1
              WHERE vault_id=?2 AND document_id=?3",
            params![
                RevisionId::new().to_string(),
                intent.vault_id.to_string(),intent.document_id.to_string(),
            ],
        ).unwrap();
        drop(c);
        let cold=SqliteSyncJournal::open_existing_read_only(
            &dir.path().join("sync.sqlite"),
        ).unwrap();
        assert!(matches!(
            cold.read_existing_replace_receipt_candidate(intent.operation_id),
            Err(JournalError::CorruptedRecord)
        ));
        assert_eq!(cold.get(intent.operation_id).unwrap().unwrap().phase,
            JournalPhase::Staged);
    }

    #[test]
    fn v7_to_v8_additive_migration_keeps_oneshot_and_baseline() {
        let (dir,mut journal,intent,protected,_candidate)=fixture();
        let claim=journal.record_existing_replace_attempt_once(&intent,&protected).unwrap();
        let baseline=journal.baseline(intent.vault_id,intent.document_id).unwrap();
        drop(journal);
        let file=dir.path().join("sync.sqlite");
        let conn=Connection::open(&file).unwrap();
        conn.execute_batch(
            "BEGIN IMMEDIATE;
             DROP TABLE sync_existing_replace_receipt_candidates;
             PRAGMA user_version = 7;
             COMMIT;",
        ).unwrap();
        drop(conn);
        assert!(matches!(
            SqliteSyncJournal::open_existing_read_only(&file),
            Err(JournalError::UnsupportedSchema(7))
        ));
        let mut upgraded=SqliteSyncJournal::open(&file).unwrap();
        assert_eq!(upgraded.baseline(intent.vault_id,intent.document_id)
            .unwrap(),baseline);
        assert_eq!(upgraded.read_existing_replace_attempt(intent.operation_id)
            .unwrap(),Some(claim));
        assert!(upgraded.read_existing_replace_receipt_candidate(intent.operation_id)
            .unwrap().is_none());
        assert!(matches!(
            upgraded.mark_applied(intent.operation_id),
            Err(JournalError::LocalDownloadNeedsNativeReceipt)
        ));
        let version:i64=upgraded.connection.query_row(
            "PRAGMA user_version",[],|row|row.get(0),
        ).unwrap();
        assert_eq!(version,8);
    }
}
