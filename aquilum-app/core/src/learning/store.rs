//! LANG-02b: durable, versioned **metadata only** for first-party study collections.
//!
//! The trusted host chooses a dedicated app-data SQLite path. This store does
//! not open notes, grant vault access, mint table IDs or touch Markdown.
//! Its source strings are locators only and must be resolved against current
//! DocumentHub/File Gate authority before assembling exercises.
//!
//! Full replacement uses an immediate SQLite transaction with a strict
//! revision CAS. It never resets or silently discards malformed old state.

use std::path::Path;

use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::collections::{self, CollectionError, Registry};

const VERSION: i64 = 1;
const MAX_BYTES: usize = 1024 * 1024;
const MAX_COLLECTIONS: usize = 512;
const MAX_TABLES: usize = 8192;
const MAX_MEMBERSHIPS: usize = 32768;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    Storage,
    UnsupportedSchema,
    CorruptStore,
    InvalidRegistry(CollectionError),
    LimitExceeded,
    StaleRevision,
    RevisionExhausted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub revision: u64,
    pub registry: Registry,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u32,
    registry: Registry,
}

pub struct RegistryStore {
    connection: Connection,
}

fn within_limits(registry: &Registry) -> bool {
    if registry.collections.len() > MAX_COLLECTIONS || registry.tables.len() > MAX_TABLES {
        return false;
    }
    let mut memberships = 0usize;
    for group in &registry.collections {
        if group.id.len() > 128 || group.name.len() > 256 {
            return false;
        }
        let Some(total) = memberships.checked_add(group.members.len()) else {
            return false;
        };
        memberships = total;
        if memberships > MAX_MEMBERSHIPS {
            return false;
        }
    }
    registry.tables.iter().all(|table| {
        table.columns.answers.len() <= 32
            && table.columns.question <= 65535
            && table.columns.context.is_none_or(|n| n <= 65535)
            && table.columns.answers.iter().all(|n| *n <= 65535)
    })
}

fn encode(registry: &Registry) -> Result<Vec<u8>, StoreError> {
    collections::validate(registry).map_err(StoreError::InvalidRegistry)?;
    if !within_limits(registry) {
        return Err(StoreError::LimitExceeded);
    }
    let bytes = serde_json::to_vec(&Envelope { version: VERSION as u32, registry: registry.clone() })
        .map_err(|_| StoreError::Storage)?;
    if bytes.len() > MAX_BYTES {
        return Err(StoreError::LimitExceeded);
    }
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Result<Registry, StoreError> {
    if bytes.len() > MAX_BYTES {
        return Err(StoreError::CorruptStore);
    }
    let data: Envelope = serde_json::from_slice(bytes).map_err(|_| StoreError::CorruptStore)?;
    if i64::from(data.version) != VERSION {
        return Err(StoreError::UnsupportedSchema);
    }
    collections::validate(&data.registry).map_err(|_| StoreError::CorruptStore)?;
    if !within_limits(&data.registry) {
        return Err(StoreError::CorruptStore);
    }
    Ok(data.registry)
}

impl RegistryStore {
    /// Only host-owned app-data paths are valid. Never use a Markdown vault
    /// path or a path obtained from a plugin / imported table cell.
    pub fn open(app_data_file: &Path) -> Result<Self, StoreError> {
        let mut connection = Connection::open(app_data_file).map_err(|_| StoreError::Storage)?;
        connection.execute_batch(
            "PRAGMA foreign_keys=ON;
             PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA busy_timeout=3000;",
        ).map_err(|_| StoreError::Storage)?;
        let schema: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|_| StoreError::Storage)?;
        if schema == 0 {
            // Refuse to take ownership of an unrelated existing database.
            let others: i64 = connection.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                [], |row| row.get(0),
            ).map_err(|_| StoreError::Storage)?;
            if others != 0 {
                return Err(StoreError::UnsupportedSchema);
            }
            let empty = encode(&Registry { collections: Vec::new(), tables: Vec::new() })?;
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| StoreError::Storage)?;
            tx.execute_batch(
                "CREATE TABLE language_registry (
                    id INTEGER PRIMARY KEY CHECK(id=1),
                    revision INTEGER NOT NULL CHECK(revision>=0),
                    payload BLOB NOT NULL
                 );",
            ).map_err(|_| StoreError::Storage)?;
            tx.execute(
                "INSERT INTO language_registry(id,revision,payload) VALUES(1,0,?1)",
                params![empty],
            ).map_err(|_| StoreError::Storage)?;
            tx.execute_batch("PRAGMA user_version=1;").map_err(|_| StoreError::Storage)?;
            tx.commit().map_err(|_| StoreError::Storage)?;
        } else if schema != VERSION {
            return Err(StoreError::UnsupportedSchema);
        }
        let store = Self { connection };
        // Never overwrite or silently reset an unsupported/corrupt file.
        store.load()?;
        Ok(store)
    }

    pub fn load(&self) -> Result<Snapshot, StoreError> {
        let (revision, payload): (i64, Vec<u8>) = self.connection.query_row(
            "SELECT revision,payload FROM language_registry WHERE id=1",
            [], |r| Ok((r.get(0)?, r.get(1)?)),
        ).map_err(|_| StoreError::CorruptStore)?;
        let revision = u64::try_from(revision).map_err(|_| StoreError::CorruptStore)?;
        Ok(Snapshot { revision, registry: decode(&payload)? })
    }

    /// Atomic compare-and-swap replacement. Caller must retain and submit
    /// the revision obtained from load(), not invent a new one.
    pub fn save(&mut self, expected_revision: u64, registry: &Registry) -> Result<u64, StoreError> {
        let payload = encode(registry)?;
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| StoreError::Storage)?;
        let current: i64 = tx.query_row(
            "SELECT revision FROM language_registry WHERE id=1", [], |r| r.get(0),
        ).map_err(|_| StoreError::CorruptStore)?;
        if u64::try_from(current).map_err(|_| StoreError::CorruptStore)? != expected_revision {
            return Err(StoreError::StaleRevision);
        }
        let next = current.checked_add(1).ok_or(StoreError::RevisionExhausted)?;
        let updated = tx.execute(
            "UPDATE language_registry SET revision=?1,payload=?2 WHERE id=1 AND revision=?3",
            params![next, payload, current],
        ).map_err(|_| StoreError::Storage)?;
        if updated != 1 {
            return Err(StoreError::StaleRevision);
        }
        tx.commit().map_err(|_| StoreError::Storage)?;
        u64::try_from(next).map_err(|_| StoreError::RevisionExhausted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{ColumnMapping, collections::{Collection, ConfiguredTable}};
    use super::super::identity::Source;

    fn registry() -> Registry {
        let source = Source {
            vault_id: "vault-1".into(), note_id: "note-42".into(), table_id: "table-7".into(),
        };
        Registry {
            collections: vec![
                Collection { id: "en".into(), name: "Английский".into(), members: vec![source.clone()] },
                Collection { id: "verbs".into(), name: "Глаголы".into(), members: vec![source.clone()] },
            ],
            tables: vec![ConfiguredTable {
                source,
                columns: ColumnMapping { question: 0, answers: vec![1, 2], context: Some(3) },
            }],
        }
    }

    #[test]
    fn reopen_preserves_memberships_without_copying_source_words() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("language-config.sqlite");
        let mut store = RegistryStore::open(&db).unwrap();
        let initial = store.load().unwrap();
        assert_eq!(initial.revision, 0);
        assert!(initial.registry.collections.is_empty());
        assert_eq!(store.save(initial.revision, &registry()), Ok(1));
        drop(store);

        let reopened = RegistryStore::open(&db).unwrap();
        assert_eq!(reopened.load(), Ok(Snapshot { revision: 1, registry: registry() }));
        let bytes = std::fs::read(&db).unwrap();
        assert!(!bytes.windows("eat".len()).any(|w| w == b"eat"));
        assert!(!bytes.windows("translation".len()).any(|w| w == b"translation"));
    }

    #[test]
    fn stale_writer_cannot_overwrite_a_saved_collection() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("study.sqlite");
        let mut one = RegistryStore::open(&db).unwrap();
        let mut two = RegistryStore::open(&db).unwrap();
        let stale_revision = two.load().unwrap().revision;
        assert_eq!(one.save(0, &registry()), Ok(1));
        assert_eq!(two.save(stale_revision, &Registry { tables: vec![], collections: vec![] }),
            Err(StoreError::StaleRevision));
        assert_eq!(two.load().unwrap().registry, registry());
    }

    #[test]
    fn invalid_registry_is_rejected_without_erasing_old_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = RegistryStore::open(&dir.path().join("study.sqlite")).unwrap();
        store.save(0, &registry()).unwrap();
        let mut broken = registry();
        broken.collections[0].members.push(broken.collections[0].members[0].clone());
        assert_eq!(
            store.save(1, &broken),
            Err(StoreError::InvalidRegistry(CollectionError::DuplicateMembership))
        );
        assert_eq!(store.load().unwrap().revision, 1);
        assert_eq!(store.load().unwrap().registry, registry());
    }

    #[test]
    fn old_or_foreign_database_is_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("foreign.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE unrelated(a TEXT);").unwrap();
        drop(connection);
        assert!(matches!(RegistryStore::open(&path), Err(StoreError::UnsupportedSchema)));
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("PRAGMA user_version=99;").unwrap();
        drop(connection);
        assert!(matches!(RegistryStore::open(&path), Err(StoreError::UnsupportedSchema)));
    }

    #[test]
    fn corrupted_json_fails_closed_and_stays_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("study.sqlite");
        let store = RegistryStore::open(&path).unwrap();
        store.connection.execute(
            "UPDATE language_registry SET payload=?1 WHERE id=1", params![b"{ bad"],
        ).unwrap();
        assert_eq!(store.load(), Err(StoreError::CorruptStore));
        drop(store);
        assert!(matches!(RegistryStore::open(&path), Err(StoreError::CorruptStore)));
    }

    #[test]
    fn unknown_future_payload_version_fails_without_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = RegistryStore::open(&dir.path().join("study.sqlite")).unwrap();
        let registry = registry();
        store.save(0, &registry).unwrap();
        store.connection.execute(
            "UPDATE language_registry SET payload=?1 WHERE id=1",
            params![br#"{"version":2,"registry":{"collections":[],"tables":[]}}"#],
        ).unwrap();
        assert_eq!(store.load(), Err(StoreError::UnsupportedSchema));
    }

    #[test]
    fn oversized_collection_label_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = RegistryStore::open(&dir.path().join("study.sqlite")).unwrap();
        let mut data = registry();
        data.collections[0].name = "x".repeat(MAX_BYTES + 1);
        assert_eq!(store.save(0, &data), Err(StoreError::LimitExceeded));
        assert_eq!(store.load().unwrap().revision, 0);
    }
}
