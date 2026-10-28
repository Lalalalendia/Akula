//! Durable local audit only: records a separately verified Vault import.
//! A DB row is neither a remote ACK nor authorization to publish a file.
use super::*;
use std::io::Read;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportAuditStatus {
    NotRecorded,
    Verified,
    NeedsReview,
}

impl InboxLedger {
    /// Call only after Core File Gate created a new Vault file without replace.
    /// Rehash both the original receipt and destination before committing.
    pub fn record_imported(&self, id: Uuid, destination: &Path) -> Result<(), LedgerError> {
        let original = self.get(id)?.ok_or(LedgerError::UnknownTransfer)?;
        if self.status(id)? != LocalStatus::VerifiedLocal
            || self.removal_status(id)? != RemovalAuditStatus::Available
        {
            return Err(LedgerError::InvalidLocalObject);
        }
        let target = checked_destination(self, id, destination)?;
        if !verified_target(&target, &original.expected)? {
            return Err(LedgerError::InvalidLocalObject);
        }
        let path = target.to_str().ok_or(LedgerError::InvalidLocalObject)?;
        if path.len() > 4096 {
            return Err(LedgerError::InvalidLocalObject);
        }
        // Additive lazy migration, backward-compatible with read-only older DBs.
        self.db.execute_batch(
            "CREATE TABLE IF NOT EXISTS imported_inbox (
                transfer_id TEXT PRIMARY KEY NOT NULL
                    REFERENCES received_inbox(transfer_id) ON DELETE RESTRICT,
                destination TEXT NOT NULL
                    CHECK(length(destination) > 0 AND length(destination) <= 4096)
            );",
        )?;
        self.db.execute(
            "INSERT OR IGNORE INTO imported_inbox (transfer_id, destination)
             VALUES (?1, ?2)",
            params![id.to_string(), path],
        )?;
        let recorded: String = self.db.query_row(
            "SELECT destination FROM imported_inbox WHERE transfer_id = ?1",
            [id.to_string()],
            |row| row.get(0),
        )?;
        if recorded != path {
            return Err(LedgerError::ConflictingIntent);
        }
        if self.import_status(id)? != ImportAuditStatus::Verified {
            return Err(LedgerError::InvalidLocalObject);
        }
        Ok(())
    }

    /// On every read, recheck both original Inbox and copied Vault bytes.
    /// Deletion or external modification becomes NeedsReview without data loss.
    pub fn import_status(&self, id: Uuid) -> Result<ImportAuditStatus, LedgerError> {
        let exists: Option<i64> = self
            .db
            .query_row(
                "SELECT 1 FROM sqlite_master
             WHERE type = 'table' AND name = 'imported_inbox'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Ok(ImportAuditStatus::NotRecorded);
        }
        let path: Option<String> = self
            .db
            .query_row(
                "SELECT destination FROM imported_inbox WHERE transfer_id = ?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let Some(path) = path else {
            return Ok(ImportAuditStatus::NotRecorded);
        };
        let Some(original) = self.get(id)? else {
            return Ok(ImportAuditStatus::NeedsReview);
        };
        match self.removal_status(id)? {
            RemovalAuditStatus::Available if self.status(id)? == LocalStatus::VerifiedLocal => {}
            RemovalAuditStatus::Removed => {}
            _ => return Ok(ImportAuditStatus::NeedsReview),
        }
        let target = Path::new(&path);
        if checked_destination(self, id, target).is_err()
            || !verified_target(target, &original.expected)?
        {
            return Ok(ImportAuditStatus::NeedsReview);
        }
        Ok(ImportAuditStatus::Verified)
    }
}

fn checked_destination(
    ledger: &InboxLedger,
    id: Uuid,
    path: &Path,
) -> Result<PathBuf, LedgerError> {
    if id.is_nil()
        || !path.is_absolute()
        || path.file_name() != Some(std::ffi::OsStr::new(&format!("received-{id}.bin")))
    {
        return Err(LedgerError::InvalidLocalObject);
    }
    let inbox = ledger
        .objects
        .parent()
        .ok_or(LedgerError::UnsafeRoot)?
        .canonicalize()?;
    let target = path.canonicalize()?;
    if target.starts_with(inbox) {
        return Err(LedgerError::InvalidLocalObject);
    }
    Ok(target)
}

fn verified_target(path: &Path, expected: &ExpectedTransfer) -> Result<bool, LedgerError> {
    let info = match fs::symlink_metadata(path) {
        Ok(info) => info,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !info.file_type().is_file() || info.len() != expected.bytes {
        return Ok(false);
    }
    let mut target = File::open(path)?;
    if !target.metadata()?.is_file() || target.metadata()?.len() != expected.bytes {
        return Ok(false);
    }
    let mut digest = blake3::Hasher::new();
    let mut total = 0u64;
    let mut block = [0u8; BUFFER_BYTES];
    loop {
        let n = target.read(&mut block)?;
        if n == 0 {
            break;
        }
        total = total
            .checked_add(n as u64)
            .ok_or(LedgerError::InvalidLocalObject)?;
        if total > expected.bytes {
            return Ok(false);
        }
        digest.update(&block[..n]);
    }
    Ok(total == expected.bytes
        && target.metadata()?.len() == expected.bytes
        && *digest.finalize().as_bytes() == expected.blake3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture() -> (
        tempfile::TempDir,
        InboxLedger,
        ExpectedTransfer,
        Vec<u8>,
        PathBuf,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("inbox");
        let vault = dir.path().join("registered-vault");
        fs::create_dir(&vault).unwrap();
        let ledger = InboxLedger::open(&root).unwrap();
        let bytes = b"locally verified and imported".to_vec();
        let intent = ExpectedTransfer {
            id: Uuid::new_v4(),
            sender_key: [11; 32],
            recipient_key: [12; 32],
            source: Transport::P2p,
            bytes: bytes.len() as u64,
            blake3: *blake3::hash(&bytes).as_bytes(),
        };
        ledger.reserve(&intent).unwrap();
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(ledger.object_path(intent.id))
            .unwrap();
        f.write_all(&bytes).unwrap();
        f.sync_all().unwrap();
        drop(f);
        let target = vault.join(format!("received-{}.bin", intent.id));
        (dir, ledger, intent, bytes, target)
    }

    #[test]
    fn import_audit_survives_reopen_and_detects_external_change() {
        let (dir, mut ledger, intent, bytes, target) = fixture();
        assert_eq!(
            ledger.import_status(intent.id).unwrap(),
            ImportAuditStatus::NotRecorded
        );
        assert!(matches!(
            ledger.record_imported(intent.id, &target),
            Err(LedgerError::InvalidLocalObject)
        ));
        assert_eq!(ledger.confirm(&intent).unwrap(), LocalStatus::VerifiedLocal);
        fs::write(&target, &bytes).unwrap();
        ledger.record_imported(intent.id, &target).unwrap();
        assert_eq!(
            ledger.import_status(intent.id).unwrap(),
            ImportAuditStatus::Verified
        );
        drop(ledger);
        let reader = InboxLedger::open_existing(&dir.path().join("inbox"))
            .unwrap()
            .unwrap();
        assert_eq!(
            reader.import_status(intent.id).unwrap(),
            ImportAuditStatus::Verified
        );
        fs::write(&target, b"externally edited").unwrap();
        assert_eq!(
            reader.import_status(intent.id).unwrap(),
            ImportAuditStatus::NeedsReview
        );
        assert_eq!(
            reader.status(intent.id).unwrap(),
            LocalStatus::VerifiedLocal
        );
    }

    #[test]
    fn import_audit_is_idempotent_and_rejects_new_destinations() {
        let (dir, mut ledger, intent, bytes, target) = fixture();
        ledger.confirm(&intent).unwrap();
        fs::write(&target, &bytes).unwrap();
        ledger.record_imported(intent.id, &target).unwrap();
        ledger.record_imported(intent.id, &target).unwrap();
        let other = dir.path().join("second-vault");
        fs::create_dir(&other).unwrap();
        let second = other.join(format!("received-{}.bin", intent.id));
        fs::write(&second, &bytes).unwrap();
        assert!(matches!(
            ledger.record_imported(intent.id, &second),
            Err(LedgerError::ConflictingIntent)
        ));
        assert_eq!(
            ledger.import_status(intent.id).unwrap(),
            ImportAuditStatus::Verified
        );
        assert_eq!(fs::read(ledger.object_path(intent.id)).unwrap(), bytes);
    }

    #[test]
    fn verified_vault_copy_survives_explicit_inbox_removal() {
        let (_dir, mut ledger, intent, bytes, target) = fixture();
        ledger.confirm(&intent).unwrap();
        fs::write(&target, &bytes).unwrap();
        ledger.record_imported(intent.id, &target).unwrap();
        assert_eq!(
            ledger.import_status(intent.id).unwrap(),
            ImportAuditStatus::Verified
        );
        ledger.request_removal(intent.id).unwrap();
        assert_eq!(
            ledger.import_status(intent.id).unwrap(),
            ImportAuditStatus::NeedsReview
        );
        fs::remove_file(ledger.object_path(intent.id)).unwrap();
        ledger.finish_removal(intent.id).unwrap();
        assert_eq!(
            ledger.import_status(intent.id).unwrap(),
            ImportAuditStatus::Verified
        );
        assert_eq!(fs::read(&target).unwrap(), bytes);
        fs::write(&target, b"tampered after removal").unwrap();
        assert_eq!(
            ledger.import_status(intent.id).unwrap(),
            ImportAuditStatus::NeedsReview
        );
    }

    #[test]
    fn import_audit_does_not_claim_missing_or_tampered_files() {
        let (_dir, mut ledger, intent, bytes, target) = fixture();
        ledger.confirm(&intent).unwrap();
        fs::write(&target, &bytes).unwrap();
        ledger.record_imported(intent.id, &target).unwrap();
        fs::remove_file(&target).unwrap();
        assert_eq!(
            ledger.import_status(intent.id).unwrap(),
            ImportAuditStatus::NeedsReview
        );
        fs::write(&target, &bytes).unwrap();
        fs::write(ledger.object_path(intent.id), b"untrusted").unwrap();
        assert_eq!(
            ledger.import_status(intent.id).unwrap(),
            ImportAuditStatus::NeedsReview
        );
    }
}
