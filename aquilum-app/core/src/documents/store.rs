use super::resolve::{SyncPoint, SyncPointLookup};
use rusqlite::{params, Connection, OptionalExtension};
use std::fmt;
use std::path::Path;

const SCHEMA_VERSION: i64 = 2;

#[derive(Debug)]
pub enum StoreError {
    Sqlite(rusqlite::Error),
    UnsupportedSchema(i64),
    HoldPresent,
    HoldMissing,
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "{error}"),
            Self::UnsupportedSchema(version) => {
                write!(formatter, "documents store schema {version} is newer than this build")
            }
            Self::HoldPresent => formatter.write_str("document has an unresolved native write intent"),
            Self::HoldMissing => formatter.write_str("durable native write intent is missing or changed"),
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

pub type StoreResult<T> = Result<T, StoreError>;

/// A crash-resumable, fail-closed pre-publication intent. It is NOT a
/// confirmation that bytes or HistoryService were modified. Never infer a
/// successful publication from this row or release it automatically.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DurableWriteIntent {
    pub(crate) base_hash: String,
    pub(crate) proposed_hash: String,
}

pub struct DocumentStore {
    connection: Connection,
}

impl DocumentStore {
    pub fn open(path: &Path) -> StoreResult<Self> {
        Self::prepare(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn memory() -> StoreResult<Self> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(connection: Connection) -> StoreResult<Self> {
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 2000;",
        )?;
        let version = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
        match version {
            0 => connection.execute_batch(&format!(
                "CREATE TABLE replicas (key TEXT PRIMARY KEY, state BLOB NOT NULL);
                 CREATE TABLE replica_updates (
                     seq INTEGER PRIMARY KEY AUTOINCREMENT,
                     key TEXT NOT NULL,
                     data BLOB NOT NULL
                 );
                 CREATE INDEX replica_updates_key ON replica_updates (key, seq);
                 CREATE TABLE sync_points (
                     key TEXT PRIMARY KEY,
                     file_hash TEXT NOT NULL,
                     text_hash TEXT NOT NULL
                 );
                 CREATE TABLE write_intents (
                     key TEXT PRIMARY KEY,
                     base_hash TEXT NOT NULL,
                     proposed_hash TEXT NOT NULL
                 );
                 PRAGMA user_version = {SCHEMA_VERSION};"
            ))?,
            1 => connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE IF NOT EXISTS write_intents (
                     key TEXT PRIMARY KEY,
                     base_hash TEXT NOT NULL,
                     proposed_hash TEXT NOT NULL
                 );
                 PRAGMA user_version = 2;
                 COMMIT;"
            )?,
            SCHEMA_VERSION => {}
            newer => return Err(StoreError::UnsupportedSchema(newer)),
        }
        Ok(Self { connection })
    }

    /// Inspect durable HOLD evidence before reading/merging the live disk.
    /// The caller MUST propagate query failure; absent rows mean no durable
    /// write attempt was armed for this document.
    pub(crate) fn write_intent(&self, key: &str) -> StoreResult<Option<DurableWriteIntent>> {
        self.connection.query_row(
            "SELECT base_hash, proposed_hash FROM write_intents WHERE key = ?1",
            [key],
            |row| Ok(DurableWriteIntent {
                base_hash: row.get(0)?, proposed_hash: row.get(1)?,
            }),
        ).optional().map_err(StoreError::from)
    }

    /// Reserve a crash-durable ambiguity window BEFORE any native writer is
    /// called. Only an explicitly armed root uses this slower FULL-sync path;
    /// ordinary offline/editor saves retain their original cost model.
    ///
    /// This is an INSERT-only one-shot: no upsert, auto-clear, or implicit
    /// overwrite of an already unresolved attempt.
    pub(crate) fn begin_write_intent(
        &self, key: &str, base_hash: &str, proposed_hash: &str,
    ) -> StoreResult<()> {
        self.with_full_sync(|connection| {
            let written = connection.execute(
                "INSERT OR IGNORE INTO write_intents (key, base_hash, proposed_hash)
                 VALUES (?1, ?2, ?3)",
                params![key, base_hash, proposed_hash],
            )?;
            if written != 1 { return Err(StoreError::HoldPresent); }
            Ok(())
        })
    }

    /// A confirmed native write + successful HistoryService and durable
    /// sync-point update can clear *only its exact* pre-publication intent.
    /// Any failure leaves a persistent HOLD for forensic review on restart.
    pub(crate) fn complete_write_intent(
        &self, key: &str, base_hash: &str, proposed_hash: &str,
    ) -> StoreResult<()> {
        self.with_full_sync(|connection| {
            let cleared = connection.execute(
                "DELETE FROM write_intents
                 WHERE key = ?1 AND base_hash = ?2 AND proposed_hash = ?3",
                params![key, base_hash, proposed_hash],
            )?;
            if cleared != 1 { return Err(StoreError::HoldMissing); }
            Ok(())
        })
    }

    fn with_full_sync<T>(
        &self, operation: impl FnOnce(&Connection) -> StoreResult<T>,
    ) -> StoreResult<T> {
        // WAL + synchronous=NORMAL is appropriate for frequent CRDT deltas,
        // but NOT for an authoritative pre-write crash marker. Switch to
        // FULL for this one SQLite commit, then restore NORMAL. The DB
        // connection is already serialized behind DocumentHub.store mutex.
        self.connection.execute_batch("PRAGMA synchronous = FULL;")?;
        let result = operation(&self.connection);
        let restore = self.connection.execute_batch("PRAGMA synchronous = NORMAL;");
        match (result, restore) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (_, Err(error)) => Err(error.into()),
        }
    }

    pub fn load(&self, key: &str) -> StoreResult<Vec<Vec<u8>>> {
        let mut blobs: Vec<Vec<u8>> = self
            .connection
            .query_row("SELECT state FROM replicas WHERE key = ?1", [key], |row| row.get(0))
            .optional()?
            .into_iter()
            .collect();
        let mut statement =
            self.connection.prepare_cached("SELECT data FROM replica_updates WHERE key = ?1 ORDER BY seq")?;
        for update in statement.query_map([key], |row| row.get::<_, Vec<u8>>(0))? {
            blobs.push(update?);
        }
        Ok(blobs)
    }

    pub fn has_replica(&self, key: &str) -> StoreResult<bool> {
        let found = self.connection.query_row(
            "SELECT EXISTS (SELECT 1 FROM replicas WHERE key = ?1)
                 OR EXISTS (SELECT 1 FROM replica_updates WHERE key = ?1)",
            [key],
            |row| row.get::<_, bool>(0),
        )?;
        Ok(found)
    }

    pub fn append(&self, key: &str, update: &[u8]) -> StoreResult<()> {
        self.connection
            .prepare_cached("INSERT INTO replica_updates (key, data) VALUES (?1, ?2)")?
            .execute(params![key, update])?;
        Ok(())
    }

    pub fn compact(&mut self, key: &str, state: &[u8]) -> StoreResult<()> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO replicas (key, state) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET state = excluded.state",
            params![key, state],
        )?;
        transaction.execute("DELETE FROM replica_updates WHERE key = ?1", [key])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn forget(&mut self, key: &str) -> StoreResult<()> {
        if self.write_intent(key)?.is_some() { return Err(StoreError::HoldPresent); }
        let transaction = self.connection.transaction()?;
        forget_in(&transaction, key)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn relocate(&mut self, from: &str, to: &str) -> StoreResult<()> {
        if from == to {
            return Ok(());
        }
        if self.write_intent(from)?.is_some() || self.write_intent(to)?.is_some() {
            // A rename/relocation cannot silently discard or transfer an
            // unresolved write attempt to a different document identity.
            return Err(StoreError::HoldPresent);
        }
        let transaction = self.connection.transaction()?;
        forget_in(&transaction, to)?;
        for table in ["replicas", "replica_updates", "sync_points"] {
            transaction.execute(&format!("UPDATE {table} SET key = ?2 WHERE key = ?1"), params![from, to])?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn sync_point(&self, key: &str) -> SyncPointLookup {
        let found = self
            .connection
            .query_row(
                "SELECT file_hash, text_hash FROM sync_points WHERE key = ?1",
                [key],
                |row| Ok(SyncPoint { file_hash: row.get(0)?, text_hash: row.get(1)? }),
            )
            .optional();
        match found {
            Ok(Some(point)) => SyncPointLookup::Found(point),
            Ok(None) => SyncPointLookup::Absent,
            Err(_) => SyncPointLookup::Unavailable,
        }
    }

    pub fn remember_sync_point(&self, key: &str, point: &SyncPoint) -> StoreResult<()> {
        self.connection.execute(
            "INSERT INTO sync_points (key, file_hash, text_hash) VALUES (?1, ?2, ?3)
             ON CONFLICT (key) DO UPDATE SET file_hash = excluded.file_hash, text_hash = excluded.text_hash",
            params![key, point.file_hash, point.text_hash],
        )?;
        Ok(())
    }
}

fn forget_in(connection: &Connection, key: &str) -> rusqlite::Result<()> {
    for table in ["replicas", "replica_updates", "sync_points"] {
        connection.execute(&format!("DELETE FROM {table} WHERE key = ?1"), [key])?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
