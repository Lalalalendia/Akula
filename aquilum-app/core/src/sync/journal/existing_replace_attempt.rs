//! Durable existing-file attempt fence. NO OS mutation or baseline receipt.
//! A recovered attempt remains HOLD, and cannot be retried automatically.

use super::*;
use crate::sync::protection_store::BoundProtectedOriginal;
use rusqlite::OptionalExtension;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadOnlyExistingReplaceAttempt {
    pub operation_id: OperationId,
    pub vault_id: VaultId,
    pub document_id: DocumentId,
    pub path: SyncRelativePath,
    pub baseline_revision: RevisionId,
    pub baseline_operation: OperationId,
    pub preimage_hash: String,
    pub target_hash: String,
    pub intent_digest: String,
    pub root: PinnedFileIdentity,
    pub original_file: PinnedFileIdentity,
    pub original_length: u64,
}

impl SqliteSyncJournal {
    /// Negative UI admission only. The current recovery overview understands
    /// additive claims, not existing-file replacement or remote VaultId mapping.
    /// Any replacement row must prevent a misleading empty/partial overview.
    /// Do not load evidence_json, original bytes, or enumerate all attempts.
    /// Even a corrupt evidence payload requires inspection, never "clean".
    pub(crate) fn existing_replace_attempts_present(&self) -> JournalResult<bool> {
        let present: i64 = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sync_existing_replace_attempts LIMIT 1)",
            [], |row| row.get(0),
        )?;
        match present {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(JournalError::CorruptedRecord),
        }
    }

    /// Permanently claim one (vault, portable path, prior baseline).
    /// A different NEW baseline requires a separately proven native receipt.
    /// BoundProtectedOriginal must come from registered pinned Core capture;
    /// this method alone does not check current OS handles or grant writes.
    #[allow(dead_code)]
    pub(crate) fn record_existing_replace_attempt_once(
        &mut self,
        intent: &OperationIntent,
        protected: &BoundProtectedOriginal,
    ) -> JournalResult<ReadOnlyExistingReplaceAttempt> {
        if intent.action != JournalAction::StageDownload
            || intent.remote_guard.is_some()
            || intent.expected_baseline.is_none()
            || protected.original.operation_id != intent.operation_id
            || protected.original.path != intent.path
            || protected.root.device == 0 || protected.root.file == 0
            || protected.file.device == 0 || protected.file.file == 0
            || protected.original.bytes.len() > 32 * 1024 * 1024
            || blake3::hash(&protected.original.bytes).to_hex().as_str()
                != protected.original.content_hash
        {
            return Err(JournalError::InvalidTransition);
        }
        let tx = self.connection.transaction_with_behavior(
            TransactionBehavior::Immediate,
        )?;
        let record = read_operation(&tx,intent.operation_id)?
            .ok_or(JournalError::MissingOperation)?;
        if record.phase != JournalPhase::Staged
            || record.intent != *intent
            || record.staged_hash.as_deref()
                != Some(intent.expected_content_hash.as_str())
        {
            return Err(JournalError::InvalidTransition);
        }
        let baseline = load_baseline(&tx,intent.vault_id,intent.document_id)?
            .ok_or(JournalError::BaselineChanged)?;
        if intent.expected_baseline != Some(baseline.revision_id)
            || baseline.profile_id != intent.profile_id
            || baseline.content_hash != protected.original.content_hash
        {
            return Err(JournalError::BaselineChanged);
        }
        let first = read_operation(&tx,baseline.operation_id)?
            .ok_or(JournalError::CorruptedRecord)?;
        if first.phase != JournalPhase::Committed
            || first.intent.path != intent.path
            || first.intent.vault_id != intent.vault_id
            || first.intent.document_id != intent.document_id
            || first.intent.revision_id != baseline.revision_id
            || first.intent.profile_id != baseline.profile_id
            || first.intent.expected_content_hash != baseline.content_hash
        {
            return Err(JournalError::BaselineChanged);
        }
        let intent_digest = blake3::hash(&serde_json::to_vec(intent)?)
            .to_hex().to_string();
        let evidence = ReadOnlyExistingReplaceAttempt {
            operation_id:intent.operation_id,
            vault_id:intent.vault_id,
            document_id:intent.document_id,
            path:intent.path.clone(),
            baseline_revision:baseline.revision_id,
            baseline_operation:baseline.operation_id,
            preimage_hash:baseline.content_hash.clone(),
            target_hash:intent.expected_content_hash.clone(),
            intent_digest:intent_digest.clone(),
            root:protected.root,
            original_file:protected.file,
            original_length:protected.original.bytes.len() as u64,
        };
        let bytes = serde_json::to_vec(&evidence)?;
        let digest = blake3::hash(&bytes).to_hex().to_string();
        let key = intent.path.collision_key();
        // SQLite verifies CAS in the same write transaction as INSERT.
        // Foreign documents, including historical additive publishers, may
        // not claim the same portable destination. An attempted update of
        // the same prior revision is permanently non-replayable.
        let n = tx.execute(
            "INSERT OR IGNORE INTO sync_existing_replace_attempts
                 (operation_id,vault_id,document_id,path_collision_key,
                  baseline_revision,evidence_json,evidence_digest)
             SELECT ?1,?2,?3,?4,?5,?6,?7
               FROM sync_operations AS op
               JOIN sync_baselines AS base
                 ON base.vault_id=op.vault_id
                AND base.document_id=op.document_id
              WHERE op.operation_id=?1 AND op.phase=1
                AND op.staged_hash=?8 AND op.digest=?9
                AND base.revision_id=?5 AND base.operation_id=?10
                AND base.content_hash=?11
                AND NOT EXISTS (
                    SELECT 1 FROM sync_existing_replace_attempts AS older
                     WHERE older.vault_id=?2 AND older.path_collision_key=?4
                       AND older.document_id!=?3
                )
                AND NOT EXISTS (
                    SELECT 1 FROM sync_local_publication_attempts AS created
                    JOIN sync_operations AS owner
                      ON owner.operation_id=created.operation_id
                     WHERE created.vault_id=?2 AND created.path_collision_key=?4
                       AND owner.document_id!=?3
                )",
            params![
                intent.operation_id.to_string(),intent.vault_id.to_string(),
                intent.document_id.to_string(),key,baseline.revision_id.to_string(),
                bytes,digest,&intent.expected_content_hash,intent_digest,
                baseline.operation_id.to_string(),baseline.content_hash,
            ],
        )?;
        if n != 1 {
            return Err(JournalError::ExistingReplaceAttemptAlreadyRecorded);
        }
        tx.commit()?;
        // A post-commit verification failure is still HOLD, never retry.
        if self.read_existing_replace_attempt(intent.operation_id)?.as_ref()
            != Some(&evidence)
        {
            return Err(JournalError::CorruptedRecord);
        }
        Ok(evidence)
    }

    /// Forensic reader, not an authority/capability and not a native receipt.
    #[allow(dead_code)]
    pub(crate) fn read_existing_replace_attempt(
        &self, id: OperationId,
    ) -> JournalResult<Option<ReadOnlyExistingReplaceAttempt>> {
        let row: Option<(String,String,String,String,Vec<u8>,String)> =
            self.connection.query_row(
                "SELECT vault_id,document_id,path_collision_key,
                        baseline_revision,evidence_json,evidence_digest
                   FROM sync_existing_replace_attempts
                  WHERE operation_id=?1",
                [id.to_string()],
                |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,
                       r.get(3)?,r.get(4)?,r.get(5)?)),
            ).optional()?;
        let Some((vault,doc,path_key,rev,bytes,digest))=row else {
            return Ok(None);
        };
        if blake3::hash(&bytes).to_hex().as_str()!=digest {
            return Err(JournalError::CorruptedRecord);
        }
        let saved:ReadOnlyExistingReplaceAttempt=serde_json::from_slice(&bytes)
            .map_err(|_| JournalError::CorruptedRecord)?;
        let operation=self.get(id)?.ok_or(JournalError::CorruptedRecord)?;
        if saved.operation_id!=id
            || saved.vault_id.to_string()!=vault
            || saved.document_id.to_string()!=doc
            || saved.path.collision_key()!=path_key
            || saved.baseline_revision.to_string()!=rev
            || saved.vault_id!=operation.intent.vault_id
            || saved.document_id!=operation.intent.document_id
            || saved.path!=operation.intent.path
            || Some(saved.baseline_revision)!=operation.intent.expected_baseline
            || saved.target_hash!=operation.intent.expected_content_hash
            || saved.intent_digest!=blake3::hash(
                &serde_json::to_vec(&operation.intent)?
               ).to_hex().to_string()
            || operation.intent.action!=JournalAction::StageDownload
            || saved.preimage_hash.len()!=64
            || saved.root.device==0 || saved.root.file==0
            || saved.original_file.device==0 || saved.original_file.file==0
            || saved.original_length>32*1024*1024
        {
            return Err(JournalError::CorruptedRecord);
        }
        Ok(Some(saved))
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::protection_store::ProtectedOriginal;
    use crate::sync::{RecoveryAction, RemoteGuard};

    fn fixture() -> (
        tempfile::TempDir, SqliteSyncJournal, OperationIntent,
        BoundProtectedOriginal,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = SqliteSyncJournal::open(
            &dir.path().join("sync.sqlite"),
        ).unwrap();
        let vault = VaultId::new();
        let document = DocumentId::new();
        let path = SyncRelativePath::parse("notes/existing.md").unwrap();
        let before = b"existing original";
        let after = b"new immutable S3 bytes";
        let rev = RevisionId::new();
        let source_id = OperationId::new();
        let original = OperationIntent {
            operation_id: source_id,
            vault_id: vault, document_id: document,
            profile_id: "approved".into(), revision_id: rev,
            action: JournalAction::UploadImmutable,
            path: path.clone(), remote_object_id: "immutable/old".into(),
            remote_guard: Some(RemoteGuard::Absent),
            expected_content_hash: blake3::hash(before).to_hex().to_string(),
            expected_baseline: None,
        };
        journal.begin(original).unwrap();
        journal.mark_staged(source_id,before).unwrap();
        journal.mark_applied(source_id).unwrap();
        journal.mark_verified(source_id,before,"remote-rev").unwrap();
        journal.commit(source_id).unwrap();

        let op = OperationId::new();
        let intent = OperationIntent {
            operation_id:op,vault_id:vault,document_id:document,
            profile_id:"approved".into(),revision_id:RevisionId::new(),
            action:JournalAction::StageDownload,path:path.clone(),
            remote_object_id:"immutable/new".into(),remote_guard:None,
            expected_content_hash:blake3::hash(after).to_hex().to_string(),
            expected_baseline:Some(rev),
        };
        journal.begin(intent.clone()).unwrap();
        journal.mark_staged(op,after).unwrap();
        let protected = BoundProtectedOriginal {
            original: ProtectedOriginal {
                operation_id:op,path,
                content_hash:blake3::hash(before).to_hex().to_string(),
                bytes:before.to_vec(),
            },
            root:PinnedFileIdentity {device:1,file:23},
            file:PinnedFileIdentity {device:1,file:24},
        };
        (dir,journal,intent,protected)
    }

    #[test]
    fn fence_is_non_replayable_between_two_sqlite_connections_and_restart() {
        let (dir,mut first,intent,protected)=fixture();
        let mut other=SqliteSyncJournal::open(
            &dir.path().join("sync.sqlite"),
        ).unwrap();
        let baseline=first.baseline(intent.vault_id,intent.document_id).unwrap();
        let recorded=first.record_existing_replace_attempt_once(
            &intent,&protected,
        ).unwrap();
        assert_eq!(recorded.preimage_hash,protected.original.content_hash);
        assert!(matches!(
            other.record_existing_replace_attempt_once(&intent,&protected),
            Err(JournalError::ExistingReplaceAttemptAlreadyRecorded)
        ));
        assert_eq!(first.baseline(intent.vault_id,intent.document_id).unwrap(),baseline);
        assert!(matches!(
            first.mark_applied(intent.operation_id),
            Err(JournalError::LocalDownloadNeedsNativeReceipt)
        ));
        assert!(first.recovery_queue().unwrap().iter().any(|i|
            i.record.intent.operation_id==intent.operation_id
            && i.action==RecoveryAction::InspectBeforeRetry
        ));
        drop(other);
        drop(first);
        let mut restarted=SqliteSyncJournal::open(
            &dir.path().join("sync.sqlite"),
        ).unwrap();
        assert_eq!(restarted.read_existing_replace_attempt(intent.operation_id)
            .unwrap(),Some(recorded));
        assert!(matches!(
            restarted.record_existing_replace_attempt_once(&intent,&protected),
            Err(JournalError::ExistingReplaceAttemptAlreadyRecorded)
        ));
    }

    #[test]
    fn wrong_original_path_bytes_and_os_identity_never_claim() {
        let (_dir,mut journal,intent,protected)=fixture();
        let mut wrong=protected.clone();
        wrong.original.path=SyncRelativePath::parse("notes/other.md").unwrap();
        assert!(journal.record_existing_replace_attempt_once(&intent,&wrong).is_err());
        wrong=protected.clone();
        wrong.original.bytes[0] ^= 1;
        assert!(journal.record_existing_replace_attempt_once(&intent,&wrong).is_err());
        wrong=protected.clone();
        wrong.root.file=0;
        assert!(journal.record_existing_replace_attempt_once(&intent,&wrong).is_err());
        wrong=protected.clone();
        wrong.original.content_hash=blake3::hash(b"other").to_hex().to_string();
        assert!(journal.record_existing_replace_attempt_once(&intent,&wrong).is_err());
        assert!(journal.read_existing_replace_attempt(intent.operation_id)
            .unwrap().is_none());
        journal.record_existing_replace_attempt_once(&intent,&protected).unwrap();
    }

    #[test]
    fn foreign_document_claiming_same_portable_path_is_held() {
        let (_dir,mut journal,intent,protected)=fixture();
        journal.record_existing_replace_attempt_once(&intent,&protected).unwrap();
        let doc2=DocumentId::new();
        let old2=b"other document";
        let new2=b"other new bytes";
        let rev2=RevisionId::new();
        let oid2=OperationId::new();
        let previous=OperationIntent {
            operation_id:oid2,vault_id:intent.vault_id,document_id:doc2,
            profile_id:"approved".into(),revision_id:rev2,
            action:JournalAction::UploadImmutable,path:intent.path.clone(),
            remote_object_id:"immutable/other".into(),
            remote_guard:Some(RemoteGuard::Absent),
            expected_content_hash:blake3::hash(old2).to_hex().to_string(),
            expected_baseline:None,
        };
        journal.begin(previous).unwrap();
        journal.mark_staged(oid2,old2).unwrap();
        journal.mark_applied(oid2).unwrap();
        journal.mark_verified(oid2,old2,"provider").unwrap();
        journal.commit(oid2).unwrap();
        let op2=OperationId::new();
        let other=OperationIntent {
            operation_id:op2,vault_id:intent.vault_id,document_id:doc2,
            profile_id:"approved".into(),revision_id:RevisionId::new(),
            action:JournalAction::StageDownload,path:intent.path.clone(),
            remote_object_id:"immutable/other-new".into(),remote_guard:None,
            expected_content_hash:blake3::hash(new2).to_hex().to_string(),
            expected_baseline:Some(rev2),
        };
        journal.begin(other.clone()).unwrap();
        journal.mark_staged(op2,new2).unwrap();
        let protected2=BoundProtectedOriginal {
            original: ProtectedOriginal {
                operation_id:op2,path:intent.path.clone(),
                content_hash:blake3::hash(old2).to_hex().to_string(),
                bytes:old2.to_vec(),
            },
            root:protected.root,file:protected.file,
        };
        assert!(matches!(
            journal.record_existing_replace_attempt_once(&other,&protected2),
            Err(JournalError::ExistingReplaceAttemptAlreadyRecorded)
        ));
        assert!(journal.read_existing_replace_attempt(op2).unwrap().is_none());
    }

    #[test]
    fn damaged_forensic_record_fails_closed_after_reopen() {
        let (dir,mut journal,intent,protected)=fixture();
        journal.record_existing_replace_attempt_once(&intent,&protected).unwrap();
        drop(journal);
        let conn=rusqlite::Connection::open(dir.path().join("sync.sqlite")).unwrap();
        conn.execute(
            "UPDATE sync_existing_replace_attempts SET evidence_digest='tampered'
              WHERE operation_id=?1",
            [intent.operation_id.to_string()],
        ).unwrap();
        drop(conn);
        let journal=SqliteSyncJournal::open(&dir.path().join("sync.sqlite")).unwrap();
        assert!(matches!(
            journal.read_existing_replace_attempt(intent.operation_id),
            Err(JournalError::CorruptedRecord)
        ));
    }

    #[test]
    fn v6_schema_upgrades_without_losing_staged_operation_or_baseline() {
        let (dir,journal,intent,_protected)=fixture();
        let baseline=journal.baseline(intent.vault_id,intent.document_id).unwrap();
        drop(journal);
        let file=dir.path().join("sync.sqlite");
        let conn=rusqlite::Connection::open(&file).unwrap();
        conn.execute_batch(
            "BEGIN IMMEDIATE;
             DROP TABLE sync_existing_replace_attempts;
             PRAGMA user_version = 6;
             COMMIT;",
        ).unwrap();
        drop(conn);
        assert!(matches!(
            SqliteSyncJournal::open_existing_read_only(&file),
            Err(JournalError::UnsupportedSchema(6))
        ));
        let migrated=SqliteSyncJournal::open(&file).unwrap();
        assert_eq!(migrated.baseline(intent.vault_id,intent.document_id)
            .unwrap(),baseline);
        assert_eq!(migrated.get(intent.operation_id).unwrap().unwrap().phase,
            JournalPhase::Staged);
        assert!(migrated.read_existing_replace_attempt(intent.operation_id)
            .unwrap().is_none());
        let v:i64=migrated.connection.query_row(
            "PRAGMA user_version",[],|row|row.get(0),
        ).unwrap();
        assert_eq!(v,7);
    }
}
