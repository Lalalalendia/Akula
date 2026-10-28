//! Durable two-phase removal intent. No physical file deletion occurs here.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemovalAuditStatus {
    Available,
    Requested,
    Removed,
    NeedsReview,
}

impl InboxLedger {
    /// Local user intent only. Future OS deletion must commit this first.
    pub fn request_removal(&self, id: Uuid) -> Result<(), LedgerError> {
        if id.is_nil() || self.status(id)? != LocalStatus::VerifiedLocal
            || self.removal_status(id)? != RemovalAuditStatus::Available
        {
            return Err(LedgerError::InvalidLocalObject);
        }
        let (file, _) = self.open_verified_file(id)?;
        drop(file);
        self.db.execute_batch(
            "CREATE TABLE IF NOT EXISTS removal_inbox (
                transfer_id TEXT PRIMARY KEY NOT NULL
                    REFERENCES received_inbox(transfer_id) ON DELETE RESTRICT,
                state INTEGER NOT NULL CHECK(state IN (0, 1))
            );",
        )?;
        self.db.execute(
            "INSERT OR IGNORE INTO removal_inbox (transfer_id, state) VALUES (?1, 0)",
            [id.to_string()],
        )?;
        if self.removal_status(id)? != RemovalAuditStatus::Requested {
            return Err(LedgerError::ConflictingIntent);
        }
        Ok(())
    }

    /// Does not remove files. The OS-scoped worker must have already removed
    /// the exact pinned object; a crash between steps remains NeedsReview.
    pub fn finish_removal(&self, id: Uuid) -> Result<(), LedgerError> {
        if !matches!(self.removal_row(id)?, Some(0) | Some(1)) {
            return Err(LedgerError::UnknownTransfer);
        }
        match fs::symlink_metadata(self.object_path(id)) {
            Ok(_) => return Err(LedgerError::InvalidLocalObject),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        self.db.execute(
            "UPDATE removal_inbox SET state = 1 WHERE transfer_id = ?1",
            [id.to_string()],
        )?;
        Ok(())
    }

    pub fn removal_status(&self, id: Uuid) -> Result<RemovalAuditStatus, LedgerError> {
        if id.is_nil() {
            return Err(LedgerError::InvalidIntent);
        }
        let Some(state) = self.removal_row(id)? else {
            return Ok(RemovalAuditStatus::Available);
        };
        match fs::symlink_metadata(self.object_path(id)) {
            Ok(_) if state == 1 => Ok(RemovalAuditStatus::NeedsReview),
            Ok(_) if self.status(id)? == LocalStatus::VerifiedLocal => {
                Ok(RemovalAuditStatus::Requested)
            }
            Ok(_) => Ok(RemovalAuditStatus::NeedsReview),
            Err(e) if e.kind() == io::ErrorKind::NotFound && state == 1 => {
                Ok(RemovalAuditStatus::Removed)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                Ok(RemovalAuditStatus::NeedsReview)
            }
            Err(e) => Err(e.into()),
        }
    }

    fn removal_row(&self, id: Uuid) -> Result<Option<i64>, LedgerError> {
        let exists: Option<i64> = self.db.query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='removal_inbox'",
            [],
            |row| row.get(0),
        ).optional()?;
        if exists.is_none() {
            return Ok(None);
        }
        let state: Option<i64> = self.db.query_row(
            "SELECT state FROM removal_inbox WHERE transfer_id = ?1",
            [id.to_string()],
            |row| row.get(0),
        ).optional()?;
        if state.is_some_and(|v| !matches!(v, 0 | 1)) {
            return Err(LedgerError::CorruptDatabaseRecord);
        }
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn durable_request_never_unlinks_and_requires_explicit_finish() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("inbox");
        let mut ledger = InboxLedger::open(&root).unwrap();
        let bytes = b"locally authenticated data";
        let intent = ExpectedTransfer {
            id: Uuid::new_v4(),
            sender_key: [7; 32],
            recipient_key: [8; 32],
            source: Transport::P2p,
            bytes: bytes.len() as u64,
            blake3: *blake3::hash(bytes).as_bytes(),
        };
        ledger.reserve(&intent).unwrap();
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(ledger.object_path(intent.id))
            .unwrap();
        f.write_all(bytes).unwrap();
        f.sync_all().unwrap();
        drop(f);
        ledger.confirm(&intent).unwrap();
        assert_eq!(ledger.removal_status(intent.id).unwrap(), RemovalAuditStatus::Available);
        ledger.request_removal(intent.id).unwrap();
        assert_eq!(ledger.removal_status(intent.id).unwrap(), RemovalAuditStatus::Requested);
        assert!(ledger.open_verified_file(intent.id).is_err());
        assert!(ledger.finish_removal(intent.id).is_err());
        assert_eq!(fs::read(ledger.object_path(intent.id)).unwrap(), bytes);
        drop(ledger);
        let reader = InboxLedger::open_existing(&root).unwrap().unwrap();
        assert_eq!(reader.removal_status(intent.id).unwrap(), RemovalAuditStatus::Requested);
        drop(reader);
        let ledger = InboxLedger::open(&root).unwrap();
        fs::remove_file(ledger.object_path(intent.id)).unwrap();
        assert_eq!(ledger.removal_status(intent.id).unwrap(), RemovalAuditStatus::NeedsReview);
        ledger.finish_removal(intent.id).unwrap();
        assert_eq!(ledger.removal_status(intent.id).unwrap(), RemovalAuditStatus::Removed);
        fs::write(ledger.object_path(intent.id), b"wrong").unwrap();
        assert_eq!(ledger.removal_status(intent.id).unwrap(), RemovalAuditStatus::NeedsReview);
    }
}
