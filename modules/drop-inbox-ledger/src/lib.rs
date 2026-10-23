#![forbid(unsafe_code)]
//! Local, provider-neutral receipt ledger for a future Akula Received Inbox.
//!
//! This crate does not trust cloud metadata, authenticate senders, decrypt files,
//! transport bytes, create user files, change the Vault, or issue sender ACKs.
//! A caller MUST separately authenticate the sender, recipient, transfer ID,
//! size and BLAKE3 commitment before constructing `ExpectedTransfer`.
//!
//! The file writer (currently `p2p_inbox_lab::save_verified_stage_as_new`)
//! must create a no-replace, verified file at `object_path` before `confirm`.
//! This ledger never creates, renames, replaces or removes Inbox objects.
//!
//! `VerifiedLocal` means the DB record AND a freshly opened local file match.
//! It is not a signed remote delivery ACK or proof against power loss. The
//! higher-level host must handle OS-owned root custody, ACLs, Windows reparse
//! points, installation, backup, and crash/power-loss acceptance separately.

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const MAX_ITEM_BYTES: u64 = 8 * 1024 * 1024;
const BUFFER_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    P2p,
    S3,
    Dropbox,
}

impl Transport {
    fn as_str(self) -> &'static str {
        match self {
            Self::P2p => "p2p",
            Self::S3 => "s3",
            Self::Dropbox => "dropbox",
        }
    }
}

/// These values must originate from a locally verified Akula signed intent,
/// not a remote URL, Dropbox rev, S3 VersionId, HTTP response or filename.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedTransfer {
    pub id: Uuid,
    pub sender_key: [u8; 32],
    pub recipient_key: [u8; 32],
    pub source: Transport,
    pub bytes: u64,
    pub blake3: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalStatus {
    Unknown,
    Pending,
    VerifiedLocal,
    NeedsReview,
}

#[derive(Debug)]
pub enum LedgerError {
    InvalidIntent,
    UnsafeRoot,
    ConflictingIntent,
    UnknownTransfer,
    InvalidLocalObject,
    CorruptDatabaseRecord,
    Io(io::Error),
    Database(rusqlite::Error),
}

impl From<io::Error> for LedgerError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<rusqlite::Error> for LedgerError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Database(value)
    }
}

pub struct InboxLedger {
    db: Connection,
    objects: PathBuf,
}

impl InboxLedger {
    /// The caller is responsible for independently authenticating and pinning
    /// the local root. This lab rejects symlinks on the root and direct objects
    /// directory, but does not claim to handle hostile ancestor-directory races.
    pub fn open(root: &Path) -> Result<Self, LedgerError> {
        if !root.exists() {
            fs::create_dir_all(root)?;
        }
        if !regular_directory(root)? {
            return Err(LedgerError::UnsafeRoot);
        }
        let objects = root.join("objects");
        if !objects.exists() {
            fs::create_dir(&objects)?;
        }
        if !regular_directory(&objects)? {
            return Err(LedgerError::UnsafeRoot);
        }
        let db_path = root.join("received-inbox.sqlite3");
        if let Ok(meta) = fs::symlink_metadata(&db_path) {
            if !meta.file_type().is_file() {
                return Err(LedgerError::UnsafeRoot);
            }
        }
        let db = Connection::open(db_path)?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS received_inbox (
                transfer_id TEXT PRIMARY KEY NOT NULL,
                sender BLOB NOT NULL CHECK(length(sender) = 32),
                recipient BLOB NOT NULL CHECK(length(recipient) = 32),
                source TEXT NOT NULL CHECK(source IN ('p2p', 's3', 'dropbox')),
                bytes INTEGER NOT NULL CHECK(bytes > 0 AND bytes <= 8388608),
                digest BLOB NOT NULL CHECK(length(digest) = 32),
                state INTEGER NOT NULL CHECK(state IN (0, 1))
             );",
        )?;
        Ok(Self { db, objects })
    }

    /// Open an already-existing Inbox without creating an empty SQLite file or
    /// allowing application SQL writes. Reader may inspect only checked bytes.
    pub fn open_existing(root: &Path) -> Result<Option<Self>, LedgerError> {
        match fs::symlink_metadata(root) {
            Ok(meta) if !meta.file_type().is_dir() => return Err(LedgerError::UnsafeRoot),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
        let objects = root.join("objects");
        if !regular_directory(&objects)? {
            return Err(LedgerError::UnsafeRoot);
        }
        let db_path = root.join("received-inbox.sqlite3");
        match fs::symlink_metadata(&db_path) {
            Ok(meta) if !meta.file_type().is_file() => return Err(LedgerError::UnsafeRoot),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
        let db = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        db.pragma_update(None, "query_only", "ON")?;
        Ok(Some(Self { db, objects }))
    }

    /// The newest bounded page by local SQLite insertion order, not a
    /// provider-controlled timestamp. Every row is independently rehashed.
    pub fn list_recent(
        &self,
        max_items: usize,
    ) -> Result<Vec<(ExpectedTransfer, LocalStatus)>, LedgerError> {
        if max_items == 0 || max_items > 64 {
            return Err(LedgerError::InvalidIntent);
        }
        let mut stmt = self
            .db
            .prepare("SELECT transfer_id FROM received_inbox ORDER BY rowid DESC LIMIT ?1")?;
        let ids = stmt.query_map([max_items as i64], |row| row.get::<_, String>(0))?;
        let mut result = Vec::new();
        for id in ids {
            let id = Uuid::parse_str(&id?).map_err(|_| LedgerError::CorruptDatabaseRecord)?;
            let record = self.get(id)?.ok_or(LedgerError::CorruptDatabaseRecord)?;
            result.push((record.expected, self.status(id)?));
        }
        Ok(result)
    }

    /// Internal transfer ID only. Remote filenames are never used as paths.
    pub fn object_path(&self, transfer_id: Uuid) -> PathBuf {
        self.objects
            .join(format!("{}.blob", transfer_id.hyphenated()))
    }

    /// Persist immutable intent *before* the file writer publishes its object.
    /// Duplicate grants cannot change the sender, recipient, transport or hash.
    pub fn reserve(&self, expected: &ExpectedTransfer) -> Result<LocalStatus, LedgerError> {
        validate(expected)?;
        self.db.execute(
            "INSERT OR IGNORE INTO received_inbox
                (transfer_id, sender, recipient, source, bytes, digest, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            params![
                expected.id.to_string(),
                &expected.sender_key[..],
                &expected.recipient_key[..],
                expected.source.as_str(),
                expected.bytes as i64,
                &expected.blake3[..]
            ],
        )?;
        let saved = self
            .get(expected.id)?
            .ok_or(LedgerError::CorruptDatabaseRecord)?;
        if saved.expected != *expected {
            return Err(LedgerError::ConflictingIntent);
        }
        self.status(expected.id)
    }

    /// Bridge between an independently authenticated offer and a trusted
    /// create-NO-REPLACE writer supplied by Akula Core.
    ///
    /// The host MUST independently verify ExpectedTransfer from a signed offer.
    /// The writer is trusted application code, not remote/provider-supplied code.
    /// Provider HTTP 200 and remote ACK cannot authorize writing or import.
    /// Existing files are reverified and never overwritten on retries.
    pub fn receive_with_writer<F>(
        &mut self,
        expected: &ExpectedTransfer,
        writer: F,
    ) -> Result<LocalStatus, LedgerError>
    where
        F: FnOnce(&Path) -> Result<(), LedgerError>,
    {
        match self.reserve(expected)? {
            LocalStatus::VerifiedLocal => return Ok(LocalStatus::VerifiedLocal),
            LocalStatus::NeedsReview => return Err(LedgerError::InvalidLocalObject),
            LocalStatus::Pending => {}
            LocalStatus::Unknown => return Err(LedgerError::CorruptDatabaseRecord),
        }
        let destination = self.object_path(expected.id);
        match fs::symlink_metadata(&destination) {
            // May have been saved before a crash: verify the same fixed object.
            // Existing symlinks, directories and altered files fail closed.
            Ok(_) => return self.confirm(expected),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        match writer(&destination) {
            Ok(()) => {}
            // Another trusted no-replace publisher can win between the
            // existence check and this writer. Never accept its return code
            // alone: independently confirm the exact local bytes and intent.
            Err(LedgerError::Io(err)) if err.kind() == io::ErrorKind::AlreadyExists => {
                return self.confirm(expected);
            }
            Err(err) => return Err(err),
        }
        self.confirm(expected)
    }

    /// Caller publishes a verified file through independent create-NO-REPLACE
    /// File Gate / P2P Inbox writer, then calls `confirm` to establish local
    /// metadata. A provider HTTP 200 or S3/Dropbox ACK must NEVER call this.
    pub fn confirm(&mut self, expected: &ExpectedTransfer) -> Result<LocalStatus, LedgerError> {
        validate(expected)?;
        let saved = self.get(expected.id)?.ok_or(LedgerError::UnknownTransfer)?;
        if saved.expected != *expected {
            return Err(LedgerError::ConflictingIntent);
        }
        if !self.local_matches(expected)? {
            return Err(LedgerError::InvalidLocalObject);
        }
        let transaction = self.db.transaction()?;
        transaction.execute(
            "UPDATE received_inbox SET state = 1 WHERE transfer_id = ?1 AND state IN (0,1)",
            [expected.id.to_string()],
        )?;
        transaction.commit()?;
        // Do not trust the SQL row alone; reopen and rehash after the commit.
        self.status(expected.id)
    }

    /// On every query, a committed DB row is insufficient. The locally opened
    /// file must still match the *previously authenticated* immutable intent.
    pub fn status(&self, id: Uuid) -> Result<LocalStatus, LedgerError> {
        let Some(record) = self.get(id)? else {
            return Ok(LocalStatus::Unknown);
        };
        if record.state == 0 {
            // A previous failed writer may have left an occupied corrupt object.
            // Showing ordinary Pending would conceal a non-retryable conflict.
            match fs::symlink_metadata(self.object_path(id)) {
                Ok(_) if !self.local_matches(&record.expected)? => {
                    return Ok(LocalStatus::NeedsReview)
                }
                Ok(_) => return Ok(LocalStatus::Pending), // valid pre-commit crash candidate
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(LocalStatus::Pending),
                Err(e) => return Err(e.into()),
            }
        }
        if self.local_matches(&record.expected)? {
            Ok(LocalStatus::VerifiedLocal)
        } else {
            Ok(LocalStatus::NeedsReview)
        }
    }

    pub fn count(&self) -> Result<u64, LedgerError> {
        let n: i64 = self
            .db
            .query_row("SELECT COUNT(*) FROM received_inbox", [], |row| row.get(0))?;
        u64::try_from(n).map_err(|_| LedgerError::CorruptDatabaseRecord)
    }

    /// Checked Inbox listing. Never expose a row as received from DB state
    /// alone: each candidate is independently re-opened and rehashed.
    pub fn list(&self) -> Result<Vec<(ExpectedTransfer, LocalStatus)>, LedgerError> {
        let mut stmt = self
            .db
            .prepare("SELECT transfer_id FROM received_inbox ORDER BY transfer_id")?;
        let ids = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut result = Vec::new();
        for item in ids {
            let id_text = item?;
            let id = Uuid::parse_str(&id_text).map_err(|_| LedgerError::CorruptDatabaseRecord)?;
            let record = self.get(id)?.ok_or(LedgerError::CorruptDatabaseRecord)?;
            let checked = self.status(id)?;
            result.push((record.expected, checked));
        }
        Ok(result)
    }

    fn get(&self, id: Uuid) -> Result<Option<Record>, LedgerError> {
        let raw = self
            .db
            .query_row(
                "SELECT sender, recipient, source, bytes, digest, state
               FROM received_inbox WHERE transfer_id = ?1",
                [id.to_string()],
                |r| {
                    Ok((
                        r.get::<_, Vec<u8>>(0)?,
                        r.get::<_, Vec<u8>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, Vec<u8>>(4)?,
                        r.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((sender, recipient, source, bytes, digest, state)) = raw else {
            return Ok(None);
        };
        let source = match source.as_str() {
            "p2p" => Transport::P2p,
            "s3" => Transport::S3,
            "dropbox" => Transport::Dropbox,
            _ => return Err(LedgerError::CorruptDatabaseRecord),
        };
        let bytes = u64::try_from(bytes).map_err(|_| LedgerError::CorruptDatabaseRecord)?;
        let expected = ExpectedTransfer {
            id,
            sender_key: sender
                .try_into()
                .map_err(|_| LedgerError::CorruptDatabaseRecord)?,
            recipient_key: recipient
                .try_into()
                .map_err(|_| LedgerError::CorruptDatabaseRecord)?,
            source,
            bytes,
            blake3: digest
                .try_into()
                .map_err(|_| LedgerError::CorruptDatabaseRecord)?,
        };
        validate(&expected)?;
        if !matches!(state, 0 | 1) {
            return Err(LedgerError::CorruptDatabaseRecord);
        }
        Ok(Some(Record { expected, state }))
    }

    fn local_matches(&self, expected: &ExpectedTransfer) -> Result<bool, LedgerError> {
        let path = self.object_path(expected.id);
        let info = match fs::symlink_metadata(&path) {
            Ok(value) => value,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        if !info.file_type().is_file() || info.len() != expected.bytes {
            return Ok(false);
        }
        let mut file = File::open(&path)?;
        if !file.metadata()?.is_file() {
            return Ok(false);
        }
        let mut digest = blake3::Hasher::new();
        let mut read = 0u64;
        let mut buffer = [0u8; BUFFER_BYTES];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            read = read
                .checked_add(n as u64)
                .ok_or(LedgerError::InvalidLocalObject)?;
            if read > expected.bytes {
                return Ok(false);
            }
            digest.update(&buffer[..n]);
        }
        Ok(read == expected.bytes && *digest.finalize().as_bytes() == expected.blake3)
    }
}

/// Canonical per-user Inbox location shared by the one-file receiver and
/// experimental native UI; never accepts a provider or sender path.
pub fn user_inbox_root() -> Result<PathBuf, LedgerError> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or(LedgerError::UnsafeRoot)?;
    #[cfg(not(windows))]
    let base = match std::env::var_os("XDG_DATA_HOME") {
        Some(value) => PathBuf::from(value),
        None => PathBuf::from(std::env::var_os("HOME").ok_or(LedgerError::UnsafeRoot)?)
            .join(".local/share"),
    };
    if !base.is_absolute() {
        return Err(LedgerError::UnsafeRoot);
    }
    Ok(base.join("Akula").join("received-inbox"))
}

struct Record {
    expected: ExpectedTransfer,
    state: i64,
}

fn regular_directory(path: &Path) -> Result<bool, io::Error> {
    Ok(fs::symlink_metadata(path)?.file_type().is_dir())
}

fn validate(v: &ExpectedTransfer) -> Result<(), LedgerError> {
    if v.id.is_nil()
        || v.bytes == 0
        || v.bytes > MAX_ITEM_BYTES
        || v.sender_key == [0; 32]
        || v.recipient_key == [0; 32]
        || v.blake3 == [0; 32]
    {
        return Err(LedgerError::InvalidIntent);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
}
