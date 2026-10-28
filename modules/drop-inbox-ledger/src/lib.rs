#![forbid(unsafe_code)]
//! Local, provider-neutral receipt ledger for a future Akula Received Inbox.
//! This crate does not trust cloud metadata, authenticate senders, decrypt files,
//! transport bytes, create user files, change the Vault, or issue sender ACKs.
//! A caller MUST separately authenticate the sender, recipient, transfer ID,
//! size and BLAKE3 commitment before constructing `ExpectedTransfer`.
//! The file writer (currently `p2p_inbox_lab::save_verified_stage_as_new`)
//! must create a no-replace, verified file at `object_path` before `confirm`.
//! This ledger never creates, renames, replaces or removes Inbox objects.
//! `VerifiedLocal` means the DB record AND a freshly opened local file match.
//! It is not a signed remote delivery ACK or proof against power loss. The
//! higher-level host must handle OS-owned root custody, ACLs, Windows reparse
//! points, installation, backup, and crash/power-loss acceptance separately.

mod import_audit;
pub use import_audit::ImportAuditStatus;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
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

    /// Obtain a single pinned local file for a user-selected export. The
    /// SQLite claim alone never suffices: rehash *this opened handle*. A
    /// separate no-clobber export must rehash once more while copying.
    /// No network path, untrusted filename or Vault write is accepted here.
    pub fn open_verified_file(&self, id: Uuid) -> Result<(File, ExpectedTransfer), LedgerError> {
        let record = self.get(id)?.ok_or(LedgerError::UnknownTransfer)?;
        if record.state != 1 {
            return Err(LedgerError::InvalidLocalObject);
        }
        let expected = record.expected;
        let path = self.object_path(id);
        let info = fs::symlink_metadata(&path)?;
        if !info.file_type().is_file() || info.len() != expected.bytes {
            return Err(LedgerError::InvalidLocalObject);
        }
        let mut file = File::open(&path)?;
        let opened = file.metadata()?;
        if !opened.is_file() || opened.len() != expected.bytes {
            return Err(LedgerError::InvalidLocalObject);
        }
        let mut hasher = blake3::Hasher::new();
        let mut counted = 0u64;
        let mut buffer = [0u8; BUFFER_BYTES];
        loop {
            let size = file.read(&mut buffer)?;
            if size == 0 {
                break;
            }
            counted = counted
                .checked_add(size as u64)
                .ok_or(LedgerError::InvalidLocalObject)?;
            if counted > expected.bytes {
                return Err(LedgerError::InvalidLocalObject);
            }
            hasher.update(&buffer[..size]);
        }
        if counted != expected.bytes
            || file.metadata()?.len() != expected.bytes
            || *hasher.finalize().as_bytes() != expected.blake3
        {
            return Err(LedgerError::InvalidLocalObject);
        }
        file.seek(SeekFrom::Start(0))?;
        Ok((file, expected))
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
mod tests;
