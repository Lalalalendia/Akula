//! Non-destructive QA journal for a FRESH local recovery review.
//! An old Requested deletion never supplies fresh consent. This journal does
//! not delete bytes, create a kernel disposition witness, or finish removal.
use super::*;

/// Evidence that an independent local UI must collect afresh from the user.
/// Neither the stored Requested row nor a network command supplies this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalRecoveryApproval {
    pub transfer_id: Uuid,
    pub bytes: u64,
    pub digest: [u8; 32],
    pub nonce: [u8; 16],
}

impl InboxLedger {
    /// Claim the ONE allowed review attempt. A Windows caller must independently
    /// open/pin the actual current OS object to supply observed_identity.
    /// The SQLite layer only compares evidence; it cannot attest to the OS
    /// handle or user interaction. Even SUCCESS here authorizes NO unlink.
    pub fn record_one_shot_recovery_review(
        &self,
        approval: LocalRecoveryApproval,
        observed_identity: RemovalOriginalIdentity,
    ) -> Result<(), LedgerError> {
        let id = approval.transfer_id;
        if id.is_nil()
            || approval.bytes == 0
            || approval.bytes > MAX_ITEM_BYTES
            || approval.digest == [0; 32]
            || approval.nonce == [0; 16]
            || observed_identity.volume_serial == 0
            || observed_identity.file_index == 0
            || observed_identity.hard_links != 1
            || self.removal_status(id)? != RemovalAuditStatus::Requested
            || self.status(id)? != LocalStatus::VerifiedLocal
            || self.original_identity_for_manual_review(id)? != Some(observed_identity)
        {
            return Err(LedgerError::InvalidLocalObject);
        }
        let record = self.get(id)?.ok_or(LedgerError::UnknownTransfer)?;
        if record.state != 1
            || record.expected.bytes != approval.bytes
            || record.expected.blake3 != approval.digest
        {
            return Err(LedgerError::InvalidLocalObject);
        }
        // On legacy requests no origin record exists, therefore this table
        // cannot authorize attempts. A primary key is a persistent one-shot CAS.
        self.db.execute_batch(
            "CREATE TABLE IF NOT EXISTS removal_recovery_review (
                transfer_id TEXT PRIMARY KEY NOT NULL
                    REFERENCES removal_original_identity(transfer_id) ON DELETE RESTRICT,
                local_nonce BLOB NOT NULL CHECK(length(local_nonce) = 16),
                original_volume INTEGER NOT NULL
                    CHECK(original_volume BETWEEN 1 AND 4294967295),
                original_file_index BLOB NOT NULL
                    CHECK(length(original_file_index) = 8),
                observed_bytes INTEGER NOT NULL
                    CHECK(observed_bytes BETWEEN 1 AND 8388608),
                observed_digest BLOB NOT NULL CHECK(length(observed_digest) = 32)
            );",
        )?;
        // The final SQLite INSERT is atomic across competing connections.
        // Re-check the original immutable receipt and identity in SQL itself.
        let inserted = self.db.execute(
            "INSERT OR IGNORE INTO removal_recovery_review
                (transfer_id, local_nonce, original_volume, original_file_index,
                 observed_bytes, observed_digest)
             SELECT r.transfer_id, ?2, i.volume_serial, i.file_index,
                    t.bytes, t.digest
             FROM removal_inbox AS r
             JOIN removal_original_identity AS i ON i.transfer_id = r.transfer_id
             JOIN received_inbox AS t ON t.transfer_id = r.transfer_id
             WHERE r.transfer_id = ?1 AND r.state = 0 AND t.state = 1
               AND t.bytes = ?3 AND t.digest = ?4
               AND i.volume_serial = ?5 AND i.file_index = ?6
               AND i.hard_links = 1",
            params![
                id.to_string(),
                approval.nonce.to_vec(),
                i64::try_from(approval.bytes).map_err(|_| LedgerError::InvalidIntent)?,
                approval.digest.to_vec(),
                i64::from(observed_identity.volume_serial),
                observed_identity.file_index.to_le_bytes().to_vec(),
            ],
        )?;
        if inserted != 1 {
            return Err(LedgerError::ConflictingIntent);
        }
        Ok(())
    }

    /// Cold-read diagnostic for manual review only: NOT a grant to unlink.
    pub fn has_recorded_recovery_review(&self, id: Uuid) -> Result<bool, LedgerError> {
        if id.is_nil() {
            return Err(LedgerError::InvalidIntent);
        }
        let exists: Option<i64> = self
            .db
            .query_row(
                "SELECT 1 FROM sqlite_master
                 WHERE type = 'table' AND name = 'removal_recovery_review'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Ok(false);
        }
        let recorded: Option<i64> = self
            .db
            .query_row(
                "SELECT 1 FROM removal_recovery_review WHERE transfer_id = ?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(recorded.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::{Arc, Barrier},
    };

    fn fixture(
        with_identity: bool,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        ExpectedTransfer,
        RemovalOriginalIdentity,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("inbox");
        let bytes = b"fresh local review must not delete existing bytes";
        let receipt = ExpectedTransfer {
            id: Uuid::new_v4(),
            sender_key: [61; 32],
            recipient_key: [62; 32],
            source: Transport::P2p,
            bytes: bytes.len() as u64,
            blake3: *blake3::hash(bytes).as_bytes(),
        };
        let mut ledger = InboxLedger::open(&root).unwrap();
        ledger.reserve(&receipt).unwrap();
        fs::write(ledger.object_path(receipt.id), bytes).unwrap();
        assert_eq!(
            ledger.confirm(&receipt).unwrap(),
            LocalStatus::VerifiedLocal
        );
        ledger.request_removal(receipt.id).unwrap();
        let origin = RemovalOriginalIdentity {
            volume_serial: 23,
            file_index: 421,
            hard_links: 1,
        };
        if with_identity {
            ledger
                .record_original_identity_after_request(receipt.id, origin)
                .unwrap();
        }
        (dir, root, receipt, origin)
    }

    fn approval(receipt: &ExpectedTransfer) -> LocalRecoveryApproval {
        LocalRecoveryApproval {
            transfer_id: receipt.id,
            bytes: receipt.bytes,
            digest: receipt.blake3,
            nonce: [91; 16],
        }
    }

    #[test]
    fn legacy_and_stale_evidence_cannot_claim_review_or_unlink() {
        let (_dir, root, receipt, original) = fixture(false);
        let ledger = InboxLedger::open(&root).unwrap();
        assert!(ledger
            .record_one_shot_recovery_review(approval(&receipt), original,)
            .is_err());
        assert!(!ledger.has_recorded_recovery_review(receipt.id).unwrap());
        assert_eq!(
            ledger.removal_status(receipt.id).unwrap(),
            RemovalAuditStatus::Requested
        );
        let bytes = fs::read(ledger.object_path(receipt.id)).unwrap();
        assert_eq!(*blake3::hash(&bytes).as_bytes(), receipt.blake3);
        drop(ledger);

        let (_dir, root, receipt, original) = fixture(true);
        let ledger = InboxLedger::open(&root).unwrap();
        let mut bad = approval(&receipt);
        bad.bytes += 1;
        assert!(ledger
            .record_one_shot_recovery_review(bad, original)
            .is_err());
        bad = approval(&receipt);
        bad.digest = [7; 32];
        assert!(ledger
            .record_one_shot_recovery_review(bad, original)
            .is_err());
        bad = approval(&receipt);
        bad.nonce = [0; 16];
        assert!(ledger
            .record_one_shot_recovery_review(bad, original)
            .is_err());
        bad = approval(&receipt);
        bad.transfer_id = Uuid::new_v4();
        assert!(ledger
            .record_one_shot_recovery_review(bad, original)
            .is_err());
        let wrong = RemovalOriginalIdentity {
            file_index: original.file_index + 1,
            ..original
        };
        assert!(ledger
            .record_one_shot_recovery_review(approval(&receipt), wrong)
            .is_err());
        assert!(!ledger.has_recorded_recovery_review(receipt.id).unwrap());
        assert!(ledger
            .record_one_shot_recovery_review(approval(&receipt), original)
            .is_ok());
        assert!(ledger
            .record_one_shot_recovery_review(approval(&receipt), original)
            .is_err());
        assert_eq!(
            ledger.removal_status(receipt.id).unwrap(),
            RemovalAuditStatus::Requested
        );
        let saved = fs::read(ledger.object_path(receipt.id)).unwrap();
        assert_eq!(*blake3::hash(&saved).as_bytes(), receipt.blake3);
        drop(ledger);
        assert!(InboxLedger::open_existing(&root)
            .unwrap()
            .unwrap()
            .has_recorded_recovery_review(receipt.id)
            .unwrap());
    }

    #[test]
    fn two_sqlite_connections_cannot_claim_same_review() {
        let (_dir, root, receipt, original) = fixture(true);
        let first = InboxLedger::open(&root).unwrap();
        let second = InboxLedger::open(&root).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let a1 = approval(&receipt);
        let a2 = LocalRecoveryApproval {
            nonce: [92; 16],
            ..a1
        };
        let gate1 = Arc::clone(&barrier);
        let gate2 = Arc::clone(&barrier);
        let t1 = std::thread::spawn(move || {
            gate1.wait();
            first.record_one_shot_recovery_review(a1, original).is_ok()
        });
        let t2 = std::thread::spawn(move || {
            gate2.wait();
            second.record_one_shot_recovery_review(a2, original).is_ok()
        });
        assert_ne!(t1.join().unwrap(), t2.join().unwrap());
        let cold = InboxLedger::open_existing(&root).unwrap().unwrap();
        assert!(cold.has_recorded_recovery_review(receipt.id).unwrap());
        assert_eq!(
            cold.removal_status(receipt.id).unwrap(),
            RemovalAuditStatus::Requested
        );
        assert_eq!(
            fs::read(cold.object_path(receipt.id)).unwrap().len() as u64,
            receipt.bytes
        );
    }
}
