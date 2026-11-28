//! Durable metadata-only sidecar for host-owned table anchor evidence.
//!
//! This dedicated app-data database does not write Markdown, read a vault,
//! resolve a note path or authorize a plugin. The host must provide stable
//! vault/note IDs, verified table snapshots and independent access checks.
//! Stored content consists only of IDs and full-table BLAKE3 fingerprints.
//!
//! The separate store is deliberate for the current prototype. Collection
//! membership and anchor writes are NOT cross-database atomic; the host must
//! fail closed when either side lacks matching, authorized evidence.

use std::path::Path;

use rusqlite::Connection;
#[cfg(test)] use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::table_anchors::{AnchorError, NoteAnchors, validate_anchors};

const SCHEMA_VERSION: i64 = 1;
const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnchorStoreError {
    Storage,
    UnsupportedSchema,
    CorruptStore,
    InvalidAnchors(AnchorError),
    TooLarge,
    StaleRevision,
    RevisionExhausted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnchorSnapshot {
    pub revision: u64,
    pub notes: Vec<NoteAnchors>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u32,
    notes: Vec<NoteAnchors>,
}

pub struct AnchorStore {
    connection: Connection,
}

fn serialize(notes: &[NoteAnchors]) -> Result<Vec<u8>, AnchorStoreError> {
    validate_anchors(notes).map_err(AnchorStoreError::InvalidAnchors)?;
    let payload = serde_json::to_vec(&Envelope {
        version: SCHEMA_VERSION as u32,
        notes: notes.to_vec(),
    }).map_err(|_| AnchorStoreError::Storage)?;
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(AnchorStoreError::TooLarge);
    }
    Ok(payload)
}

fn deserialize(payload: &[u8]) -> Result<Vec<NoteAnchors>, AnchorStoreError> {
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(AnchorStoreError::CorruptStore);
    }
    let envelope: Envelope =
        serde_json::from_slice(payload).map_err(|_| AnchorStoreError::CorruptStore)?;
    if i64::from(envelope.version) != SCHEMA_VERSION {
        return Err(AnchorStoreError::UnsupportedSchema);
    }
    validate_anchors(&envelope.notes).map_err(|_| AnchorStoreError::CorruptStore)?;
    Ok(envelope.notes)
}

impl AnchorStore {
    /// app_data_file must be a host-controlled dedicated app-data path.
    /// A caller cannot use a note path or user-supplied file path here.
    pub fn open(app_data_file: &Path) -> Result<Self, AnchorStoreError> {
        let connection = super::sqlite_store::open(
            app_data_file, SCHEMA_VERSION,
            "CREATE TABLE language_anchors (
                    id INTEGER PRIMARY KEY CHECK(id=1),
                    revision INTEGER NOT NULL CHECK(revision>=0),
                    payload BLOB NOT NULL
                 );",
            "INSERT INTO language_anchors(id,revision,payload) VALUES(1,0,?1)",
            || serialize(&[]),
        ).map_err(|error| match error {
            super::sqlite_store::OpenError::Storage => AnchorStoreError::Storage,
            super::sqlite_store::OpenError::UnsupportedSchema => AnchorStoreError::UnsupportedSchema,
            super::sqlite_store::OpenError::Payload(error) => error,
        })?;
        let store = Self { connection };
        // Never silently erase malformed, truncated or future-version data.
        store.load()?;
        Ok(store)
    }

    pub fn load(&self) -> Result<AnchorSnapshot, AnchorStoreError> {
        let (revision, payload): (i64, Vec<u8>) = self.connection.query_row(
            "SELECT revision,payload FROM language_anchors WHERE id=1",
            [], |row| Ok((row.get(0)?, row.get(1)?)),
        ).map_err(|_| AnchorStoreError::CorruptStore)?;
        let revision = u64::try_from(revision).map_err(|_| AnchorStoreError::CorruptStore)?;
        Ok(AnchorSnapshot { revision, notes: deserialize(&payload)? })
    }

    /// Atomic full replacement using the exact revision from load().
    /// A stale process must reload and reconcile, never last-write-wins.
    pub fn save(
        &mut self,
        expected_revision: u64,
        notes: &[NoteAnchors],
    ) -> Result<u64, AnchorStoreError> {
        let payload = serialize(notes)?;
        super::sqlite_replace::replace(
            &mut self.connection, expected_revision, payload,
            "SELECT revision,payload FROM language_anchors WHERE id=1",
            "UPDATE language_anchors SET revision=?1,payload=?2
             WHERE id=1 AND revision=?3",
            deserialize, |error| match error {
                super::sqlite_replace::ReplaceError::Storage => AnchorStoreError::Storage,
                super::sqlite_replace::ReplaceError::Corrupt => AnchorStoreError::CorruptStore,
                super::sqlite_replace::ReplaceError::Stale => AnchorStoreError::StaleRevision,
                super::sqlite_replace::ReplaceError::Exhausted => AnchorStoreError::RevisionExhausted,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::table_anchors::{NoteKey, TableAnchor, fingerprint_tables};

    fn sample() -> Vec<NoteAnchors> {
        let table = vec![
            vec!["Term".to_owned(), "Definition".to_owned()],
            vec!["private-vocabulary-token".to_owned(), "секретное определение".to_owned()],
        ];
        let fingerprints = fingerprint_tables(&[table]).unwrap();
        vec![NoteAnchors {
            note: NoteKey { vault_id: "vault-1".into(), note_id: "note-17".into() },
            tables: vec![TableAnchor {
                table_id: "table-89".into(), previous_index: 0, full_digest: fingerprints[0],
            }],
        }]
    }

    #[test]
    fn survives_restart_and_does_not_store_vocabulary_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("language-table-anchors.sqlite");
        let mut store = AnchorStore::open(&path).unwrap();
        assert_eq!(store.load().unwrap().revision, 0);
        assert_eq!(store.save(0, &sample()), Ok(1));
        drop(store);
        let reopened = AnchorStore::open(&path).unwrap();
        assert_eq!(reopened.load(), Ok(AnchorSnapshot { revision: 1, notes: sample() }));
        let data = std::fs::read(&path).unwrap();
        assert!(!data.windows(b"private-vocabulary-token".len())
            .any(|chunk| chunk == b"private-vocabulary-token"));
    }

    #[test]
    fn stale_writer_cannot_replace_newer_anchor_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite");
        let mut first = AnchorStore::open(&path).unwrap();
        let mut stale = AnchorStore::open(&path).unwrap();
        assert_eq!(first.save(0, &sample()), Ok(1));
        assert_eq!(stale.save(0, &[]), Err(AnchorStoreError::StaleRevision));
        assert_eq!(stale.load().unwrap().notes, sample());
    }

    #[test]
    fn duplicate_table_identity_is_rejected_without_erasure() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = AnchorStore::open(&dir.path().join("state.sqlite")).unwrap();
        store.save(0, &sample()).unwrap();
        let mut invalid = sample();
        let mut duplicate = invalid[0].tables[0].clone();
        duplicate.previous_index = 1;
        invalid[0].tables.push(duplicate);
        assert_eq!(store.save(1, &invalid),
            Err(AnchorStoreError::InvalidAnchors(AnchorError::DuplicateTableId)));
        assert_eq!(store.load().unwrap(), AnchorSnapshot { revision: 1, notes: sample() });
    }

    #[test]
    fn unrelated_database_is_never_adopted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("foreign.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE unrelated(key TEXT);").unwrap();
        drop(connection);
        assert!(matches!(AnchorStore::open(&path), Err(AnchorStoreError::UnsupportedSchema)));
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("PRAGMA user_version=22;").unwrap();
        drop(connection);
        assert!(matches!(AnchorStore::open(&path), Err(AnchorStoreError::UnsupportedSchema)));
    }

    #[test]
    fn corrupt_payload_is_not_reset_and_cannot_be_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite");
        let mut store = AnchorStore::open(&path).unwrap();
        store.save(0, &sample()).unwrap();
        store.connection.execute(
            "UPDATE language_anchors SET payload=?1 WHERE id=1",
            params![&b"{ broken"[..]],
        ).unwrap();
        assert_eq!(store.load(), Err(AnchorStoreError::CorruptStore));
        assert_eq!(store.save(1, &[]), Err(AnchorStoreError::CorruptStore));
        drop(store);
        assert!(matches!(AnchorStore::open(&path), Err(AnchorStoreError::CorruptStore)));
    }

    #[test]
    fn future_payload_version_causes_hold_without_migration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite");
        let store = AnchorStore::open(&path).unwrap();
        store.connection.execute(
            "UPDATE language_anchors SET payload=?1 WHERE id=1",
            params![&br#"{"version":2,"notes":[]}"#[..]],
        ).unwrap();
        assert_eq!(store.load(), Err(AnchorStoreError::UnsupportedSchema));
    }

    #[test]
    fn oversized_registry_never_replaces_previous_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = AnchorStore::open(&dir.path().join("state.sqlite")).unwrap();
        let mut notes = sample();
        notes[0].tables.extend((0..9000).map(|n| TableAnchor {
            table_id: format!("table-{n}"), previous_index: n + 1, full_digest: [0; 32],
        }));
        assert_eq!(store.save(0, &notes),
            Err(AnchorStoreError::InvalidAnchors(AnchorError::TooManyAnchors)));
        assert_eq!(store.load().unwrap().revision, 0);
    }
}
