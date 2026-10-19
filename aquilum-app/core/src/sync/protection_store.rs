//! Stage 04g-3b-0: durable, inert preservation of already-verified originals.
//!
//! The backup is a SQLite BLOB in a *host-selected app-data database*, never
//! in the synced workspace. SQLite WAL + FULL synchronous protects committed
//! preimages across process restarts. Persisting bytes here is NOT a verified
//! source-file read and never authorizes a local overwrite or a VaultPort
//! PreservationReceipt. No mutation executor calls this module yet.
//!
//! The host must later bind it to the pinned root, editor admission, expected
//! on-disk hash and exclusive File Gate operation. This module cannot itself
//! prove those conditions or the integrity of the underlying storage device.

use super::pinned_read::PinnedFileIdentity;
use super::{OperationId, SyncRelativePath};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::path::Path;

const SCHEMA_VERSION: i64 = 2;
pub(crate) const MAX_PROTECTED_ORIGINAL_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug)]
pub(crate) enum ProtectionStoreError {
    Sqlite(rusqlite::Error),
    UnsupportedSchema(i64),
    WrongContentHash,
    TooLarge,
    OperationCollision,
    CorruptedRecord,
    UnboundOriginal,
    Io(std::io::Error),
}

impl From<std::io::Error> for ProtectionStoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for ProtectionStoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

pub(crate) type ProtectionResult<T> = Result<T, ProtectionStoreError>;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum ProtectionOutcome {
    Stored,
    AlreadyStored,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProtectedOriginal {
    pub operation_id: OperationId,
    pub path: SyncRelativePath,
    pub content_hash: String,
    pub bytes: Vec<u8>,
}

/// A durable provenance claim, still NOT a mutation capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoundProtectedOriginal {
    pub original: ProtectedOriginal,
    pub root: PinnedFileIdentity,
    pub file: PinnedFileIdentity,
}

/// Does not implement VaultPort, expose PreservationReceipt or hold permission
/// to write any user document. Intended only for a future trusted host bridge.
#[allow(dead_code)]
pub(crate) struct SqliteProtectionStore {
    connection: Connection,
    database_path: std::path::PathBuf,
}

#[allow(dead_code)]
impl SqliteProtectionStore {
    /// Caller supplies a trusted private app-data path OUTSIDE the workspace.
    /// Never derive this path from a remote manifest or SyncRelativePath.
    pub(crate) fn open(app_data_database: &Path) -> ProtectionResult<Self> {
        let connection = Connection::open(app_data_database)?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        match version {
            0 => connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE protected_originals (
                    operation_id TEXT PRIMARY KEY NOT NULL,
                    relative_path TEXT NOT NULL,
                    content_hash TEXT NOT NULL,
                    byte_length INTEGER NOT NULL
                        CHECK (byte_length BETWEEN 0 AND 33554432),
                    original_bytes BLOB NOT NULL
                 );
                 CREATE TABLE bound_preimages (
                    operation_id TEXT PRIMARY KEY NOT NULL
                        REFERENCES protected_originals(operation_id),
                    root_device TEXT NOT NULL,
                    root_file TEXT NOT NULL,
                    source_device TEXT NOT NULL,
                    source_file TEXT NOT NULL
                 );
                 PRAGMA user_version = 2;
                 COMMIT;",
            )?,
            1 => connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE bound_preimages (
                    operation_id TEXT PRIMARY KEY NOT NULL
                        REFERENCES protected_originals(operation_id),
                    root_device TEXT NOT NULL,
                    root_file TEXT NOT NULL,
                    source_device TEXT NOT NULL,
                    source_file TEXT NOT NULL
                 );
                 PRAGMA user_version = 2;
                 COMMIT;",
            )?,
            SCHEMA_VERSION => (),
            other => return Err(ProtectionStoreError::UnsupportedSchema(other)),
        };
        let database_path = std::fs::canonicalize(app_data_database)?;
        Ok(Self { connection, database_path })
    }

    /// Read a previously committed protected original after a cold restart.
    /// Never create or migrate an app-data database from a recovery/UI read.
    /// This object does not grant File Gate, S3 apply, or baseline authority.
    pub(crate) fn open_existing_read_only(
        app_data_database: &Path,
    ) -> ProtectionResult<Self> {
        let connection = Connection::open_with_flags(
            app_data_database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.execute_batch(
            "PRAGMA query_only = ON;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 250;",
        )?;
        let version: i64 = connection.query_row(
            "PRAGMA user_version", [], |row| row.get(0),
        )?;
        if version != SCHEMA_VERSION {
            return Err(ProtectionStoreError::UnsupportedSchema(version));
        }
        let database_path = std::fs::canonicalize(app_data_database)?;
        Ok(Self { connection, database_path })
    }

    pub(crate) fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// An atomic original+identity binding. Unbound v1 records are never
    /// promoted into "host proven" data merely because the bytes match.
    /// Only RegisteredReadAdmission may call this after pinned-file checks.
    pub(super) fn store_bound_preimage(
        &mut self,
        operation_id: OperationId,
        path: &SyncRelativePath,
        expected_hash: &str,
        bytes: &[u8],
        root: PinnedFileIdentity,
        file: PinnedFileIdentity,
    ) -> ProtectionResult<ProtectionOutcome> {
        if bytes.len() > MAX_PROTECTED_ORIGINAL_BYTES {
            return Err(ProtectionStoreError::TooLarge);
        }
        if blake3::hash(bytes).to_hex().as_str() != expected_hash {
            return Err(ProtectionStoreError::WrongContentHash);
        }
        let transaction =
            self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old = load_one(&transaction, operation_id)?;
        let outcome = if let Some(existing) = old {
            if existing.path != *path || existing.content_hash != expected_hash
                || existing.bytes != bytes
            {
                return Err(ProtectionStoreError::OperationCollision);
            }
            let provenance = load_binding(&transaction, operation_id)?;
            if provenance != Some((root, file)) {
                return Err(ProtectionStoreError::OperationCollision);
            }
            ProtectionOutcome::AlreadyStored
        } else {
            transaction.execute(
                "INSERT INTO protected_originals
                 (operation_id, relative_path, content_hash, byte_length, original_bytes)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    operation_id.to_string(), path.as_str(), expected_hash,
                    bytes.len() as i64, bytes,
                ],
            )?;
            transaction.execute(
                "INSERT INTO bound_preimages
                 (operation_id, root_device, root_file, source_device, source_file)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    operation_id.to_string(), encode_id(root.device),
                    encode_id(root.file), encode_id(file.device), encode_id(file.file)
                ],
            )?;
            ProtectionOutcome::Stored
        };
        transaction.commit()?;
        let verified = self.read_bound_preimage(operation_id)?
            .ok_or(ProtectionStoreError::CorruptedRecord)?;
        if verified.original.path != *path
            || verified.original.content_hash != expected_hash
            || verified.original.bytes != bytes
            || verified.root != root
            || verified.file != file
        {
            return Err(ProtectionStoreError::CorruptedRecord);
        }
        Ok(outcome)
    }

    /// A reloaded durable record, NOT a PreservationReceipt.
    pub(crate) fn read_bound_preimage(
        &self,
        id: OperationId,
    ) -> ProtectionResult<Option<BoundProtectedOriginal>> {
        let Some(original) = load_one(&self.connection, id)? else {
            return Ok(None);
        };
        let Some((root, file)) = load_binding(&self.connection, id)? else {
            return Err(ProtectionStoreError::UnboundOriginal);
        };
        Ok(Some(BoundProtectedOriginal { original, root, file }))
    }

    /// Persists a supplied preimage after checking BLAKE3, in a FULL-sync
    /// SQLite transaction. It does NOT assert the bytes still belong to a
    /// registered workspace file: only an OS-pinned host reader can do that.
    ///
    /// Same operation ID/path/hash/bytes is idempotent; any other reuse of
    /// this ID fails without touching the existing original.
    pub(crate) fn store_preimage(
        &mut self,
        operation_id: OperationId,
        path: &SyncRelativePath,
        expected_hash: &str,
        bytes: &[u8],
    ) -> ProtectionResult<ProtectionOutcome> {
        if bytes.len() > MAX_PROTECTED_ORIGINAL_BYTES {
            return Err(ProtectionStoreError::TooLarge);
        }
        let calculated = blake3::hash(bytes).to_hex().to_string();
        if calculated != expected_hash {
            return Err(ProtectionStoreError::WrongContentHash);
        }

        let transaction =
            self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = load_one(&transaction, operation_id)? {
            if existing.path != *path || existing.content_hash != expected_hash
                || existing.bytes != bytes
            {
                return Err(ProtectionStoreError::OperationCollision);
            }
            return Ok(ProtectionOutcome::AlreadyStored);
        }
        transaction.execute(
            "INSERT INTO protected_originals
             (operation_id, relative_path, content_hash, byte_length, original_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                operation_id.to_string(),
                path.as_str(),
                expected_hash,
                bytes.len() as i64,
                bytes,
            ],
        )?;
        transaction.commit()?;

        // The committed backup must be readable and exactly match its digest
        // before any higher-level host may rely on this persistence result.
        let after = load_one(&self.connection, operation_id)?
            .ok_or(ProtectionStoreError::CorruptedRecord)?;
        if after.path != *path || after.content_hash != expected_hash || after.bytes != bytes {
            return Err(ProtectionStoreError::CorruptedRecord);
        }
        Ok(ProtectionOutcome::Stored)
    }

    /// A validated copy, not an authority-bearing receipt.
    pub(crate) fn read_preimage(
        &self,
        operation_id: OperationId,
    ) -> ProtectionResult<Option<ProtectedOriginal>> {
        load_one(&self.connection, operation_id)
    }
}

fn encode_id(value: u64) -> String {
    format!("{value:016x}")
}

fn parse_id(raw: &str) -> ProtectionResult<u64> {
    let value = u64::from_str_radix(raw, 16)
        .map_err(|_| ProtectionStoreError::CorruptedRecord)?;
    if raw != encode_id(value) {
        return Err(ProtectionStoreError::CorruptedRecord);
    }
    Ok(value)
}

fn load_binding(
    connection: &Connection,
    id: OperationId,
) -> ProtectionResult<Option<(PinnedFileIdentity, PinnedFileIdentity)>> {
    let row: Option<(String, String, String, String)> = connection.query_row(
        "SELECT root_device, root_file, source_device, source_file
         FROM bound_preimages WHERE operation_id = ?1",
        [id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).optional()?;
    let Some((root_device, root_file, file_device, file_index)) = row else {
        return Ok(None);
    };
    Ok(Some((
        PinnedFileIdentity { device: parse_id(&root_device)?, file: parse_id(&root_file)? },
        PinnedFileIdentity { device: parse_id(&file_device)?, file: parse_id(&file_index)? },
    )))
}

fn load_one(
    connection: &Connection,
    id: OperationId,
) -> ProtectionResult<Option<ProtectedOriginal>> {
    let record: Option<(String, String, i64, Vec<u8>)> = connection.query_row(
        "SELECT relative_path, content_hash, byte_length, original_bytes
         FROM protected_originals WHERE operation_id = ?1",
        [id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).optional()?;
    let Some((raw_path, content_hash, stored_len, bytes)) = record else {
        return Ok(None);
    };
    if bytes.len() > MAX_PROTECTED_ORIGINAL_BYTES
        || stored_len != bytes.len() as i64
        || content_hash != blake3::hash(&bytes).to_hex().to_string()
    {
        return Err(ProtectionStoreError::CorruptedRecord);
    }
    let path = SyncRelativePath::parse(&raw_path)
        .map_err(|_| ProtectionStoreError::CorruptedRecord)?;
    if path.as_str() != raw_path {
        return Err(ProtectionStoreError::CorruptedRecord);
    }
    Ok(Some(ProtectedOriginal {
        operation_id: id,
        path,
        content_hash,
        bytes,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn path(raw: &str) -> SyncRelativePath {
        SyncRelativePath::parse(raw).unwrap()
    }

    #[test]
    fn protection_store_survives_reopen_without_touching_original_file() {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("appdata").join("protection.sqlite3");
        fs::create_dir_all(db.parent().unwrap()).unwrap();
        let original = root.path().join("user-document.md");
        let bytes = "Глава 140 😀\n".as_bytes();
        fs::write(&original, bytes).unwrap();
        let id = OperationId::new();
        let digest = blake3::hash(bytes).to_hex().to_string();
        let p = path("notes/Глава 140.md");
        {
            let mut store = SqliteProtectionStore::open(&db).unwrap();
            assert_eq!(
                store.store_preimage(id, &p, &digest, bytes).unwrap(),
                ProtectionOutcome::Stored
            );
        }
        let store = SqliteProtectionStore::open(&db).unwrap();
        let restored = store.read_preimage(id).unwrap().unwrap();
        assert_eq!(restored.bytes, bytes);
        assert_eq!(restored.path, p);
        assert_eq!(restored.content_hash, digest);
        assert_eq!(fs::read(original).unwrap(), bytes);
    }

    #[test]
    fn same_operation_is_idempotent_and_never_replaces_an_original() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("protection.sqlite");
        let mut store = SqliteProtectionStore::open(&db).unwrap();
        let id = OperationId::new();
        let bytes = b"first original";
        let hash = blake3::hash(bytes).to_hex().to_string();
        let p = path("notes/a.md");
        assert_eq!(store.store_preimage(id, &p, &hash, bytes).unwrap(), ProtectionOutcome::Stored);
        assert_eq!(
            store.store_preimage(id, &p, &hash, bytes).unwrap(),
            ProtectionOutcome::AlreadyStored
        );
        assert!(matches!(
            store.store_preimage(id, &path("notes/b.md"), &hash, bytes),
            Err(ProtectionStoreError::OperationCollision)
        ));
        let modified = b"other original";
        let modified_hash = blake3::hash(modified).to_hex().to_string();
        assert!(matches!(
            store.store_preimage(id, &p, &modified_hash, modified),
            Err(ProtectionStoreError::OperationCollision)
        ));
        assert_eq!(store.read_preimage(id).unwrap().unwrap().bytes, bytes);
    }

    #[test]
    fn wrong_hash_and_unbounded_payload_refuse_before_persisting() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = SqliteProtectionStore::open(&temp.path().join("p.sqlite")).unwrap();
        let id = OperationId::new();
        let p = path("notes/a.md");
        assert!(matches!(
            store.store_preimage(id, &p, "wrong", b"actual"),
            Err(ProtectionStoreError::WrongContentHash)
        ));
        let too_big = vec![0u8; MAX_PROTECTED_ORIGINAL_BYTES + 1];
        let hash = blake3::hash(&too_big).to_hex().to_string();
        assert!(matches!(
            store.store_preimage(id, &p, &hash, &too_big),
            Err(ProtectionStoreError::TooLarge)
        ));
        assert!(store.read_preimage(id).unwrap().is_none());
        let empty_hash = blake3::hash(b"").to_hex().to_string();
        assert_eq!(
            store.store_preimage(id, &p, &empty_hash, b"").unwrap(),
            ProtectionOutcome::Stored
        );
    }

    #[test]
    fn corruption_in_persisted_bytes_or_metadata_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = SqliteProtectionStore::open(&temp.path().join("p.sqlite")).unwrap();
        let id = OperationId::new();
        let data = b"protected bytes";
        let hash = blake3::hash(data).to_hex().to_string();
        store.store_preimage(id, &path("notes/a.md"), &hash, data).unwrap();
        store.connection.execute(
            "UPDATE protected_originals SET original_bytes = ?1 WHERE operation_id = ?2",
            params![b"corrupted bytes".as_slice(), id.to_string()],
        ).unwrap();
        assert!(matches!(
            store.read_preimage(id),
            Err(ProtectionStoreError::CorruptedRecord)
        ));
        assert!(matches!(
            store.store_preimage(id, &path("notes/a.md"), &hash, data),
            Err(ProtectionStoreError::CorruptedRecord)
        ));
    }

    #[test]
    fn unknown_future_schema_fails_closed_without_migrating_it() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("p.sqlite");
        let store = SqliteProtectionStore::open(&db).unwrap();
        store.connection.pragma_update(None, "user_version", 3).unwrap();
        drop(store);
        assert!(matches!(
            SqliteProtectionStore::open(&db),
            Err(ProtectionStoreError::UnsupportedSchema(3))
        ));
    }

    fn opened_id(device: u64, file: u64) -> PinnedFileIdentity {
        PinnedFileIdentity { device, file }
    }

    #[test]
    fn bound_preimages_are_atomically_recorded_and_verified_after_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("p.sqlite");
        let id = OperationId::new();
        let p = path("notes/abc.md");
        let data = b"original\n";
        let hash = blake3::hash(data).to_hex().to_string();
        let root = opened_id(u64::MAX, 7);
        let file = opened_id(42, u64::MAX);
        {
            let mut store = SqliteProtectionStore::open(&db).unwrap();
            assert_eq!(
                store.store_bound_preimage(id, &p, &hash, data, root, file).unwrap(),
                ProtectionOutcome::Stored
            );
            assert_eq!(
                store.store_bound_preimage(id, &p, &hash, data, root, file).unwrap(),
                ProtectionOutcome::AlreadyStored
            );
            assert!(matches!(
                store.store_bound_preimage(id, &p, &hash, data, root, opened_id(42, 6)),
                Err(ProtectionStoreError::OperationCollision)
            ));
            assert!(matches!(
                store.store_bound_preimage(id, &p, &hash, data, opened_id(9, 9), file),
                Err(ProtectionStoreError::OperationCollision)
            ));
        }
        let store = SqliteProtectionStore::open(&db).unwrap();
        let original = store.read_bound_preimage(id).unwrap().unwrap();
        assert_eq!(original.original.bytes, data);
        assert_eq!(original.root, root);
        assert_eq!(original.file, file);
    }

    #[test]
    fn bound_preimages_refuse_legacy_promotions_and_migrate_v1_losslessly() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("legacy.sqlite");
        let id = OperationId::new();
        let p = path("notes/legacy.md");
        let bytes = b"legacy protected";
        let hash = blake3::hash(bytes).to_hex().to_string();
        {
            let connection = Connection::open(&db).unwrap();
            connection.execute_batch(
                "CREATE TABLE protected_originals (
                    operation_id TEXT PRIMARY KEY NOT NULL,
                    relative_path TEXT NOT NULL,
                    content_hash TEXT NOT NULL,
                    byte_length INTEGER NOT NULL,
                    original_bytes BLOB NOT NULL);
                 PRAGMA user_version = 1;",
            ).unwrap();
            connection.execute(
                "INSERT INTO protected_originals
                 (operation_id, relative_path, content_hash, byte_length, original_bytes)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id.to_string(), p.as_str(), &hash, bytes.len() as i64, bytes],
            ).unwrap();
        }
        let mut store = SqliteProtectionStore::open(&db).unwrap();
        assert_eq!(
            store.connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0)).unwrap(),
            2
        );
        assert_eq!(store.read_preimage(id).unwrap().unwrap().bytes, bytes);
        assert!(matches!(
            store.read_bound_preimage(id), Err(ProtectionStoreError::UnboundOriginal)
        ));
        assert!(matches!(
            store.store_bound_preimage(id, &p, &hash, bytes, opened_id(1, 2), opened_id(3, 4)),
            Err(ProtectionStoreError::OperationCollision)
        ));
        assert_eq!(store.read_preimage(id).unwrap().unwrap().bytes, bytes);
        let fresh = OperationId::new();
        assert_eq!(
            store.store_bound_preimage(fresh, &p, &hash, bytes, opened_id(1, 2), opened_id(3, 4)).unwrap(),
            ProtectionOutcome::Stored
        );
    }

    #[test]
    fn corrupt_bound_identity_is_never_trusted() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = SqliteProtectionStore::open(&temp.path().join("p.sqlite")).unwrap();
        let id = OperationId::new();
        let bytes = b"original";
        let hash = blake3::hash(bytes).to_hex().to_string();
        store.store_bound_preimage(
            id, &path("notes/a.md"), &hash, bytes, opened_id(1, 2), opened_id(3, 4)
        ).unwrap();
        store.connection.execute(
            "UPDATE bound_preimages SET source_file = 'not-a-file-id' WHERE operation_id = ?1",
            [id.to_string()],
        ).unwrap();
        assert!(matches!(
            store.read_bound_preimage(id), Err(ProtectionStoreError::CorruptedRecord)
        ));
        assert!(matches!(
            store.store_bound_preimage(
                id, &path("notes/a.md"), &hash, bytes, opened_id(1, 2), opened_id(3, 4)
            ),
            Err(ProtectionStoreError::CorruptedRecord)
        ));

    }

    #[test]
    fn cold_open_existing_original_is_read_only_and_survives_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let database = tmp.path().join("cold.sqlite");
        let operation = OperationId::new();
        let original = b"original bytes for recovery";
        let hash = blake3::hash(original).to_hex().to_string();
        let root = opened_id(15, 17);
        let source = opened_id(15, 19);
        {
            let mut writer = SqliteProtectionStore::open(&database).unwrap();
            writer.store_bound_preimage(
                operation, &path("folder/a.md"), &hash, original, root, source,
            ).unwrap();
        }

        let cold = SqliteProtectionStore::open_existing_read_only(&database).unwrap();
        let kept = cold.read_bound_preimage(operation).unwrap().unwrap();
        assert_eq!(kept.original.bytes, original);
        assert_eq!(kept.original.content_hash, hash);
        assert_eq!(kept.root, root);
        assert_eq!(kept.file, source);
        assert!(cold.connection.execute(
            "DELETE FROM protected_originals WHERE operation_id = ?1",
            [operation.to_string()],
        ).is_err(), "a cold read must not delete protected original");
        let again = SqliteProtectionStore::open_existing_read_only(&database).unwrap();
        assert!(again.read_bound_preimage(operation).unwrap().is_some());
    }

    #[test]
    fn cold_open_missing_or_legacy_schema_must_not_create_or_migrate() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("not-created.sqlite");
        assert!(SqliteProtectionStore::open_existing_read_only(&missing).is_err());
        assert!(!missing.exists(), "read-only recovery created an evidence file");

        let legacy = tmp.path().join("legacy-v1.sqlite");
        let connection = Connection::open(&legacy).unwrap();
        connection.execute_batch(
            "PRAGMA user_version = 1;
             CREATE TABLE protected_originals (
                 operation_id TEXT PRIMARY KEY NOT NULL,
                 relative_path TEXT NOT NULL,
                 content_hash TEXT NOT NULL,
                 byte_length INTEGER NOT NULL,
                 original_bytes BLOB NOT NULL
             );",
        ).unwrap();
        drop(connection);
        assert!(matches!(
            SqliteProtectionStore::open_existing_read_only(&legacy),
            Err(ProtectionStoreError::UnsupportedSchema(1))
        ));
        let connection = Connection::open(&legacy).unwrap();
        let version: i64 = connection.query_row(
            "PRAGMA user_version", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(version, 1);
        let migrated: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='bound_preimages'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(migrated, 0);
    }
}
