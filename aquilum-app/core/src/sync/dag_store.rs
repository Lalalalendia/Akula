//! Stage 02b: durable, append-only *local* cache for the per-document revision DAG.
//!
//! This is a separate application-owned SQLite database. It is NOT the sync
//! operation journal (PR 03), and is never an authority to remove local files.
//! No cloud provider is contacted or enabled by constructing this type.

use super::dag::{DagError, IngestOutcome, RevisionDag, RevisionOperation, RevisionRecord};
use super::RevisionId;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::path::Path;

const SCHEMA_VERSION: i64 = 1;

#[derive(Debug)]
pub enum DagStoreError {
    Sqlite(rusqlite::Error),
    InvalidRecord(serde_json::Error),
    InvalidGraph(DagError),
    UnsupportedSchema(i64),
    IntegrityMismatch,
    MissingBlob,
    InvalidHash,
    UnsafeStoreLocation,
}

impl From<rusqlite::Error> for DagStoreError {
    fn from(value: rusqlite::Error) -> Self { Self::Sqlite(value) }
}

impl From<serde_json::Error> for DagStoreError {
    fn from(value: serde_json::Error) -> Self { Self::InvalidRecord(value) }
}

impl From<DagError> for DagStoreError {
    fn from(value: DagError) -> Self { Self::InvalidGraph(value) }
}

pub type DagStoreResult<T> = Result<T, DagStoreError>;

pub struct SqliteDagStore {
    connection: Connection,
}

impl SqliteDagStore {
    /// The caller provides a distinct app-data path; never point this at the
    /// existing editor DocumentStore database.
    pub fn open(path: &Path) -> DagStoreResult<Self> {
        Self::prepare(Connection::open(path)?)
    }

    /// Negative host-admission check: no SQLite cache or WAL/SHM sidecar
    /// may be stored inside a user-controlled vault. This does not prove
    /// ownership of the external app-data directory.
    pub(crate) fn require_appdata_outside_vault(&self, root: &Path)
        -> DagStoreResult<()>
    {
        let mut stmt = self.connection.prepare("PRAGMA database_list")?;
        let mut rows = stmt.query([])?;
        let mut path = None;
        while let Some(row) = rows.next()? {
            if row.get::<_, String>(1)? == "main" {
                path = Some(row.get::<_, String>(2)?);
                break;
            }
        }
        let raw = path.ok_or(DagStoreError::UnsafeStoreLocation)?;
        if raw.is_empty() {
            return Err(DagStoreError::UnsafeStoreLocation);
        }
        let file = std::fs::canonicalize(raw)
            .map_err(|_| DagStoreError::UnsafeStoreLocation)?;
        let root = std::fs::canonicalize(root)
            .map_err(|_| DagStoreError::UnsafeStoreLocation)?;
        if file.starts_with(&root) {
            return Err(DagStoreError::UnsafeStoreLocation);
        }
        let meta = std::fs::metadata(&file)
            .map_err(|_| DagStoreError::UnsafeStoreLocation)?;
        if !meta.is_file() {
            return Err(DagStoreError::UnsafeStoreLocation);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.nlink() != 1 {
                return Err(DagStoreError::UnsafeStoreLocation);
            }
        }
        #[cfg(windows)]
        {
            let handle = std::fs::File::open(&file)
                .map_err(|_| DagStoreError::UnsafeStoreLocation)?;
            let (_, _, links) = super::pinned_windows::file_identity_with_links(&handle)
                .map_err(|_| DagStoreError::UnsafeStoreLocation)?;
            if links != 1 {
                return Err(DagStoreError::UnsafeStoreLocation);
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn memory() -> DagStoreResult<Self> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(connection: Connection) -> DagStoreResult<Self> {
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
                 CREATE TABLE sync_revisions (
                     id TEXT PRIMARY KEY NOT NULL,
                     document_id TEXT NOT NULL,
                     payload BLOB NOT NULL,
                     digest TEXT NOT NULL
                 );
                 CREATE INDEX sync_revisions_document ON sync_revisions(document_id);
                 CREATE TABLE sync_blobs (
                     hash TEXT PRIMARY KEY NOT NULL,
                     data BLOB NOT NULL
                 );
                 PRAGMA user_version = 1;
                 COMMIT;",
            )?,
            SCHEMA_VERSION => {},
            other => return Err(DagStoreError::UnsupportedSchema(other)),
        }
        Ok(Self { connection })
    }

    /// A transaction validates all existing records + ancestry BEFORE the
    /// immutable new record is added. A rejected record cannot mutate the DB.
    /// Missing parents remain pending and cannot appear as active heads.
    pub fn ingest(&mut self, record: RevisionRecord) -> DagStoreResult<IngestOutcome> {
        let transaction = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut dag = load_dag_from(&transaction)?;
        let outcome = dag.ingest(record.clone())?;
        if outcome != IngestOutcome::AlreadyPresent {
            let payload = serde_json::to_vec(&record)?;
            let digest = blake3::hash(&payload).to_hex().to_string();
            transaction.execute(
                "INSERT INTO sync_revisions (id, document_id, payload, digest)
                 VALUES (?1, ?2, ?3, ?4)",
                params![record.id.to_string(), record.document_id.to_string(), payload, digest],
            )?;
        }
        transaction.commit()?;
        Ok(outcome)
    }

    /// Replay is order-independent. It fails closed on damaged records, IDs,
    /// checksum mismatches, cycles, and unsupported/malformed representations.
    pub fn load_dag(&self) -> DagStoreResult<RevisionDag> {
        load_dag_from(&self.connection)
    }

    /// Store canonical blob bytes under BLAKE3; never replace a blob in-place.
    /// PR 05 will add bounded streaming for large binary objects.
    pub fn store_blob(&mut self, bytes: &[u8]) -> DagStoreResult<String> {
        let hash = blake3::hash(bytes).to_hex().to_string();
        let transaction = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<Vec<u8>> = transaction.query_row(
            "SELECT data FROM sync_blobs WHERE hash = ?1",
            [&hash], |row| row.get(0),
        ).optional()?;
        match existing {
            Some(existing) if existing != bytes => return Err(DagStoreError::IntegrityMismatch),
            Some(_) => {},
            None => {
                transaction.execute(
                    "INSERT INTO sync_blobs (hash, data) VALUES (?1, ?2)",
                    params![&hash, bytes],
                )?;
            }
        }
        transaction.commit()?;
        Ok(hash)
    }

    /// Data from the database is never trusted merely because a matching
    /// filename/key exists. Rehash on every verified read.
    pub fn verified_blob(&self, hash: &str) -> DagStoreResult<Vec<u8>> {
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err(DagStoreError::InvalidHash);
        }
        let bytes: Vec<u8> = self.connection.query_row(
            "SELECT data FROM sync_blobs WHERE hash = ?1",
            [hash], |row| row.get(0),
        ).optional()?.ok_or(DagStoreError::MissingBlob)?;
        if blake3::hash(&bytes).to_hex().as_str() != hash {
            return Err(DagStoreError::IntegrityMismatch);
        }
        Ok(bytes)
    }

    /// A revision without its fully verified ancestry/content is not readable
    /// for a future executor. A tombstone returns None; it does not unlink.
    pub fn verified_revision_content(&self, id: RevisionId) -> DagStoreResult<Option<Vec<u8>>> {
        let dag = self.load_dag()?;
        let revision = dag.get(id).ok_or(DagStoreError::InvalidGraph(DagError::UnknownRevision))?;
        match &revision.operation {
            RevisionOperation::Content { blob_hash, .. }
            | RevisionOperation::Rename { blob_hash, .. } =>
                self.verified_blob(blob_hash).map(Some),
            RevisionOperation::Tombstone { .. } => Ok(None),
        }
    }
}

fn load_dag_from(connection: &Connection) -> DagStoreResult<RevisionDag> {
    let mut statement = connection.prepare(
        "SELECT id, document_id, payload, digest FROM sync_revisions ORDER BY id"
    )?;
    let records = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Vec<u8>>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut dag = RevisionDag::new();
    for record in records {
        let (id, doc, payload, digest) = record?;
        if blake3::hash(&payload).to_hex().as_str() != digest {
            return Err(DagStoreError::IntegrityMismatch);
        }
        let parsed: RevisionRecord = serde_json::from_slice(&payload)?;
        if parsed.id.to_string() != id || parsed.document_id.to_string() != doc {
            return Err(DagStoreError::IntegrityMismatch);
        }
        dag.ingest(parsed)?;
    }
    Ok(dag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{DocumentId, RevisionId, SyncRelativePath};
    use std::collections::BTreeSet;

    fn revision(id: RevisionId, doc: DocumentId, parents: Vec<RevisionId>, bytes: &[u8]) -> RevisionRecord {
        RevisionRecord {
            id,
            document_id: doc,
            parents,
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse("notes/file.md").unwrap(),
                blob_hash: blake3::hash(bytes).to_hex().to_string(),
            },
        }
    }

    #[test]
    fn durable_cache_must_be_outside_the_registered_user_vault() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let appdata = temp.path().join("appdata");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&appdata).unwrap();
        let unsafe_cache = SqliteDagStore::open(&root.join("cache.sqlite")).unwrap();
        assert!(matches!(
            unsafe_cache.require_appdata_outside_vault(&root),
            Err(DagStoreError::UnsafeStoreLocation)
        ));
        let external = SqliteDagStore::open(&appdata.join("cache.sqlite")).unwrap();
        assert!(external.require_appdata_outside_vault(&root).is_ok());
        let memory = SqliteDagStore::memory().unwrap();
        assert!(matches!(
            memory.require_appdata_outside_vault(&root),
            Err(DagStoreError::UnsafeStoreLocation)
        ));
    }

    #[test]
    fn sqlite_reopen_reconstructs_immutable_branches_and_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync-dag.sqlite");
        let doc = DocumentId::new();
        let root = RevisionId::new();
        let one = RevisionId::new();
        let two = RevisionId::new();
        {
            let mut store = SqliteDagStore::open(&path).unwrap();
            for (id, parents, bytes) in [
                (root, vec![], b"base".as_slice()),
                (one, vec![root], b"first".as_slice()),
                (two, vec![root], b"second".as_slice()),
            ] {
                let hash = store.store_blob(bytes).unwrap();
                let record = revision(id, doc, parents, bytes);
                assert_eq!(hash, match &record.operation {
                    RevisionOperation::Content { blob_hash, .. } => blob_hash.clone(),
                    _ => unreachable!(),
                });
                store.ingest(record).unwrap();
            }
        }
        let store = SqliteDagStore::open(&path).unwrap();
        let dag = store.load_dag().unwrap();
        assert_eq!(dag.active_len(), 3);
        assert_eq!(dag.heads(doc), BTreeSet::from([one, two]));
        assert_eq!(store.verified_revision_content(one).unwrap(), Some(b"first".to_vec()));
        assert_eq!(store.verified_revision_content(two).unwrap(), Some(b"second".to_vec()));
    }

    #[test]
    fn missing_parents_remain_pending_across_restarts_until_their_parent_arrives() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dag.sqlite");
        let doc = DocumentId::new();
        let parent = RevisionId::new();
        let child = RevisionId::new();
        {
            let mut store = SqliteDagStore::open(&path).unwrap();
            assert_eq!(store.ingest(revision(child, doc, vec![parent], b"child")).unwrap(), IngestOutcome::Pending);
            let dag = store.load_dag().unwrap();
            assert_eq!(dag.pending_len(), 1);
            assert_eq!(dag.active_len(), 0);
            assert!(matches!(
                store.verified_revision_content(child),
                Err(DagStoreError::InvalidGraph(DagError::UnknownRevision))
            ));
        }
        let mut store = SqliteDagStore::open(&path).unwrap();
        assert_eq!(store.ingest(revision(parent, doc, vec![], b"parent")).unwrap(),
            IngestOutcome::Activated(vec![parent, child]));
        assert_eq!(store.load_dag().unwrap().heads(doc), BTreeSet::from([child]));
        assert!(matches!(store.verified_revision_content(child), Err(DagStoreError::MissingBlob)));
    }

    #[test]
    fn identity_collision_is_rejected_and_original_record_retained() {
        let mut store = SqliteDagStore::memory().unwrap();
        let doc = DocumentId::new();
        let id = RevisionId::new();
        let first = revision(id, doc, vec![], b"first");
        assert!(matches!(store.ingest(first.clone()), Ok(IngestOutcome::Activated(_))));
        assert_eq!(store.ingest(first.clone()).unwrap(), IngestOutcome::AlreadyPresent);
        assert!(matches!(store.ingest(revision(id, doc, vec![], b"tamper")),
            Err(DagStoreError::InvalidGraph(DagError::RevisionCollision))));
        assert_eq!(store.load_dag().unwrap().get(id), Some(&first));
    }

    #[test]
    fn corrupted_blob_and_record_are_rejected_before_read_or_execution() {
        let mut store = SqliteDagStore::memory().unwrap();
        let doc = DocumentId::new();
        let id = RevisionId::new();
        let hash = store.store_blob(b"original").unwrap();
        store.ingest(revision(id, doc, vec![], b"original")).unwrap();
        store.connection.execute("UPDATE sync_blobs SET data = ?1 WHERE hash = ?2",
            params![b"damaged".as_slice(), &hash]).unwrap();
        assert!(matches!(store.verified_revision_content(id), Err(DagStoreError::IntegrityMismatch)));
        store.connection.execute("UPDATE sync_revisions SET payload = ?1 WHERE id = ?2",
            params![b"tampered".as_slice(), id.to_string()]).unwrap();
        assert!(matches!(store.load_dag(), Err(DagStoreError::IntegrityMismatch)));
    }

    #[test]
    fn rejected_sqlite_commit_keeps_graph_unchanged_and_retries() {
        let mut store = SqliteDagStore::memory().unwrap();
        let doc = DocumentId::new();
        let root = RevisionId::new();
        let first = revision(root, doc, vec![], b"root");
        store.connection.execute_batch(
            "CREATE TRIGGER sync_insert_fail BEFORE INSERT ON sync_revisions
             BEGIN SELECT RAISE(FAIL, 'simulated disk failure'); END;",
        ).unwrap();
        assert!(matches!(store.ingest(first.clone()), Err(DagStoreError::Sqlite(_))));
        assert_eq!(store.load_dag().unwrap().active_len(), 0);
        store.connection.execute_batch("DROP TRIGGER sync_insert_fail;").unwrap();
        assert!(matches!(store.ingest(first), Ok(IngestOutcome::Activated(_))));
        assert_eq!(store.load_dag().unwrap().active_len(), 1);
    }

    #[test]
    fn future_schema_does_not_get_downgraded_or_overwritten() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("PRAGMA user_version = 99;").unwrap();
        assert!(matches!(SqliteDagStore::prepare(connection),
            Err(DagStoreError::UnsupportedSchema(99))));
    }

    #[test]
    fn dangling_child_from_another_document_is_rejected_when_parent_arrives() {
        let mut store = SqliteDagStore::memory().unwrap();
        let doc = DocumentId::new();
        let other = DocumentId::new();
        let parent = RevisionId::new();
        let child = RevisionId::new();
        store.ingest(revision(child, doc, vec![parent], b"child")).unwrap();
        assert!(matches!(store.ingest(revision(parent, other, vec![], b"wrong doc")),
            Err(DagStoreError::InvalidGraph(DagError::CrossDocumentParent))));
        let dag = store.load_dag().unwrap();
        assert_eq!(dag.pending_len(), 1);
        assert_eq!(dag.active_len(), 0);
    }
}
