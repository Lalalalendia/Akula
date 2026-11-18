//! One atomic app-data registry for language collections AND table anchors.
//!
//! It stores no Markdown cells; all source IDs and live tables are supplied
//! by the trusted host after opening the current authorized native note.
//! A hash or saved source string is never permission to read another note.

use std::path::Path;
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::ColumnMapping;
use super::collections::{self, Collection, CollectionError, ConfiguredTable, Registry};
use super::identity::Source;
use super::table_anchors::{
    self, AnchorError, NoteAnchors, NoteKey, TableAnchor, fingerprint_tables, reconcile_tables,
};

const VERSION: i64 = 1;
const MAX_PAYLOAD: usize = 1024 * 1024;

pub mod manage;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StudyState {
    pub registry: Registry,
    pub anchors: Vec<NoteAnchors>,
}

impl Default for StudyState {
    fn default() -> Self {
        Self { registry: Registry { collections: Vec::new(), tables: Vec::new() }, anchors: Vec::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stored {
    pub revision: u64,
    pub state: StudyState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateError {
    Storage,
    Corrupt,
    UnsupportedVersion,
    InvalidCollections(CollectionError),
    InvalidAnchors(AnchorError),
    OrphanSource,
    TooLarge,
    RevisionConflict,
    RevisionExhausted,
    InvalidSelection,
    DuplicateTable,
    NeedsRebind,
    MappingConflict,
    InvalidCollection,
    EmptyStudyTable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindOutcome {
    pub revision: u64,
    pub source: Source,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    version: u32,
    state: StudyState,
}

pub(crate) fn validate(state: &StudyState) -> Result<(), StateError> {
    collections::validate(&state.registry).map_err(StateError::InvalidCollections)?;
    table_anchors::validate_anchors(&state.anchors).map_err(StateError::InvalidAnchors)?;
    for table in &state.registry.tables {
        let source = &table.source;
        let found = state.anchors.iter().any(|n| {
            n.note.vault_id == source.vault_id
                && n.note.note_id == source.note_id
                && n.tables.iter().any(|a| a.table_id == source.table_id)
        });
        if !found {
            return Err(StateError::OrphanSource);
        }
    }
    Ok(())
}

fn encode(state: &StudyState) -> Result<Vec<u8>, StateError> {
    validate(state)?;
    let payload = serde_json::to_vec(&Payload { version: VERSION as u32, state: state.clone() })
        .map_err(|_| StateError::Storage)?;
    if payload.len() > MAX_PAYLOAD {
        return Err(StateError::TooLarge);
    }
    Ok(payload)
}

fn decode(bytes: &[u8]) -> Result<StudyState, StateError> {
    if bytes.len() > MAX_PAYLOAD {
        return Err(StateError::Corrupt);
    }
    let payload: Payload = serde_json::from_slice(bytes).map_err(|_| StateError::Corrupt)?;
    if i64::from(payload.version) != VERSION {
        return Err(StateError::UnsupportedVersion);
    }
    validate(&payload.state).map_err(|_| StateError::Corrupt)?;
    Ok(payload.state)
}

/// Mutate an already validated snapshot without reading disk.
///
/// selected is an index in the **complete** current note table array.
/// A previously registered table that has been edited is held rather than
/// assigned a fresh ID, because history might otherwise move incorrectly.
pub fn bind_table(
    state: &mut StudyState,
    note: &NoteKey,
    all_tables: &[Vec<Vec<String>>],
    selected: usize,
    columns: ColumnMapping,
    collection_id: &str,
    collection_name: &str,
    candidate_table_id: &str,
) -> Result<Source, StateError> {
    validate(state)?;
    if note.vault_id.trim().is_empty() || note.note_id.trim().is_empty()
        || note.vault_id.len() > 128 || note.note_id.len() > 128
    {
        return Err(StateError::InvalidSelection);
    }
    if collection_id.trim().is_empty() || collection_name.trim().is_empty()
        || collection_id.len() > 128 || collection_name.len() > 256
        || candidate_table_id.trim().is_empty() || candidate_table_id.len() > 128
    {
        return Err(StateError::InvalidCollection);
    }
    let digests = fingerprint_tables(all_tables).map_err(StateError::InvalidAnchors)?;
    let Some(&selected_digest) = digests.get(selected) else {
        return Err(StateError::InvalidSelection);
    };
    if digests.iter().filter(|&&digest| digest == selected_digest).count() != 1 {
        return Err(StateError::DuplicateTable);
    }
    let rows = &all_tables[selected];
    columns.validate(rows[0].len()).map_err(|_| StateError::InvalidSelection)?;
    let preview = super::preview_table(rows, &columns, 128)
        .map_err(|_| StateError::InvalidSelection)?;
    if preview.items.is_empty() {
        return Err(StateError::EmptyStudyTable);
    }
    // Do not alter the caller state until the entire candidate is valid.
    let mut updated = state.clone();
    let record_pos = updated.anchors.iter().position(|r| &r.note == note);
    let table_id = if let Some(pos) = record_pos {
        let matches = reconcile_tables(&updated.anchors[pos], note, all_tables)
            .map_err(StateError::InvalidAnchors)?;
        if !matches.held.is_empty() {
            return Err(StateError::NeedsRebind);
        }
        for attached in &matches.attached {
            if let Some(anchor) = updated.anchors[pos].tables.iter_mut()
                .find(|a| a.table_id == attached.table_id)
            {
                anchor.previous_index = attached.current_index;
            }
        }
        match matches.attached.iter().find(|a| a.current_index == selected) {
            Some(attached) => attached.table_id.clone(),
            None => {
                if updated.anchors[pos].tables.iter().any(|a| a.table_id == candidate_table_id) {
                    return Err(StateError::DuplicateTable);
                }
                updated.anchors[pos].tables.push(TableAnchor {
                    table_id: candidate_table_id.to_owned(),
                    previous_index: selected,
                    full_digest: selected_digest,
                });
                candidate_table_id.to_owned()
            }
        }
    } else {
        updated.anchors.push(NoteAnchors {
            note: note.clone(),
            tables: vec![TableAnchor {
                table_id: candidate_table_id.to_owned(),
                previous_index: selected,
                full_digest: selected_digest,
            }],
        });
        candidate_table_id.to_owned()
    };

    let source = Source {
        vault_id: note.vault_id.clone(),
        note_id: note.note_id.clone(),
        table_id,
    };
    if let Some(existing) = updated.registry.tables.iter().find(|table| table.source == source) {
        if existing.columns != columns {
            return Err(StateError::MappingConflict);
        }
    } else {
        updated.registry.tables.push(ConfiguredTable { source: source.clone(), columns });
    }
    if updated.registry.collections.iter().any(|group| group.id != collection_id && group.name.to_lowercase() == collection_name.to_lowercase()) { return Err(StateError::InvalidCollection); }
    if let Some(group) = updated.registry.collections.iter_mut().find(|g| g.id == collection_id) {
        if !group.members.contains(&source) {
            group.members.push(source.clone());
        }
    } else {
        updated.registry.collections.push(Collection {
            id: collection_id.to_owned(),
            name: collection_name.to_owned(),
            members: vec![source.clone()],
        });
    }
    validate(&updated)?;
    *state = updated;
    Ok(source)
}

pub struct StudyStore {
    connection: Connection,
}

impl StudyStore {
    /// Must be called only with a host-owned app-data path.
    pub fn open(path: &Path) -> Result<Self, StateError> {
        let mut connection = Connection::open(path).map_err(|_| StateError::Storage)?;
        connection.execute_batch(
            "PRAGMA foreign_keys=ON;
             PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA busy_timeout=3000;",
        ).map_err(|_| StateError::Storage)?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|_| StateError::Storage)?;
        if version == 0 {
            let existing: i64 = connection.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                [], |row| row.get(0),
            ).map_err(|_| StateError::Storage)?;
            if existing != 0 {
                return Err(StateError::UnsupportedVersion);
            }
            let empty = encode(&StudyState::default())?;
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| StateError::Storage)?;
            tx.execute_batch(
                "CREATE TABLE language_study_state (
                    id INTEGER PRIMARY KEY CHECK(id=1),
                    revision INTEGER NOT NULL CHECK(revision>=0),
                    payload BLOB NOT NULL
                 );",
            ).map_err(|_| StateError::Storage)?;
            tx.execute(
                "INSERT INTO language_study_state(id,revision,payload) VALUES(1,0,?1)",
                params![empty],
            ).map_err(|_| StateError::Storage)?;
            tx.execute_batch("PRAGMA user_version=1;")
                .map_err(|_| StateError::Storage)?;
            tx.commit().map_err(|_| StateError::Storage)?;
        } else if version != VERSION {
            return Err(StateError::UnsupportedVersion);
        }
        let store = Self { connection };
        store.load()?;
        Ok(store)
    }

    pub fn load(&self) -> Result<Stored, StateError> {
        let (rev, payload): (i64, Vec<u8>) = self.connection.query_row(
            "SELECT revision,payload FROM language_study_state WHERE id=1",
            [], |r| Ok((r.get(0)?, r.get(1)?)),
        ).map_err(|_| StateError::Corrupt)?;
        let revision = u64::try_from(rev).map_err(|_| StateError::Corrupt)?;
        Ok(Stored { revision, state: decode(&payload)? })
    }

    /// One atomic CAS protects membership + table identity from split writes.
    /// Caller must re-verify document access and live snapshot before binding.
    pub fn bind(
        &mut self,
        expected_revision: u64,
        note: &NoteKey,
        all_tables: &[Vec<Vec<String>>],
        selected: usize,
        columns: ColumnMapping,
        collection_id: &str,
        collection_name: &str,
        candidate_table_id: &str,
    ) -> Result<BindOutcome, StateError> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| StateError::Storage)?;
        let (rev, previous): (i64, Vec<u8>) = tx.query_row(
            "SELECT revision,payload FROM language_study_state WHERE id=1",
            [], |r| Ok((r.get(0)?, r.get(1)?)),
        ).map_err(|_| StateError::Corrupt)?;
        let mut state = decode(&previous)?;
        if u64::try_from(rev).map_err(|_| StateError::Corrupt)? != expected_revision {
            return Err(StateError::RevisionConflict);
        }
        let old = state.clone();
        let source = bind_table(
            &mut state, note, all_tables, selected, columns,
            collection_id, collection_name, candidate_table_id,
        )?;
        if state == old {
            return Ok(BindOutcome { revision: expected_revision, source });
        }
        let payload = encode(&state)?;
        let next = rev.checked_add(1).ok_or(StateError::RevisionExhausted)?;
        let changed = tx.execute(
            "UPDATE language_study_state SET revision=?1,payload=?2
             WHERE id=1 AND revision=?3",
            params![next, payload, rev],
        ).map_err(|_| StateError::Storage)?;
        if changed != 1 {
            return Err(StateError::RevisionConflict);
        }
        tx.commit().map_err(|_| StateError::Storage)?;
        Ok(BindOutcome {
            revision: u64::try_from(next).map_err(|_| StateError::RevisionExhausted)?,
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str) -> NoteKey {
        NoteKey { vault_id: "vault-1".into(), note_id: id.into() }
    }
    fn rows(word: &str, answer: &str) -> Vec<Vec<String>> {
        vec![vec!["Q".into(), "A".into()], vec![word.into(), answer.into()]]
    }
    fn mapping() -> ColumnMapping {
        ColumnMapping { question: 0, answers: vec![1], context: None }
    }

    #[test]
    fn atomic_add_reopen_and_deduplicate_membership() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("language-study.sqlite");
        let mut store = StudyStore::open(&path).unwrap();
        let n = note("one");
        let tables = [rows("run", "бежать")];
        let one = store.bind(0, &n, &tables, 0, mapping(), "common", "Общая база", "table-1").unwrap();
        assert_eq!(one.revision, 1);
        let two = store.bind(1, &n, &tables, 0, mapping(), "common", "Общая база", "different").unwrap();
        assert_eq!(two.revision, 1);
        assert_eq!(one.source, two.source);
        drop(store);
        let store = StudyStore::open(&path).unwrap();
        let saved = store.load().unwrap();
        assert_eq!(saved.state.registry.collections[0].members.len(), 1);
        assert_eq!(saved.state.anchors[0].tables.len(), 1);
        assert_eq!(saved.state.registry.tables.len(), 1);
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.windows("бежать".len()).any(|w| w == "бежать".as_bytes()));
    }

    #[test]
    fn two_collections_and_three_notes_share_sources_without_copies() {
        let mut state = StudyState::default();
        let n = note("one");
        let first = [rows("eat", "есть"), rows("ate", "ел")];
        let a = bind_table(&mut state, &n, &first, 0, mapping(), "en", "English", "a").unwrap();
        bind_table(&mut state, &n, &first, 0, mapping(), "verbs", "Verbs", "other").unwrap();
        assert_eq!(state.registry.collections.len(), 2);
        assert_eq!(state.registry.collections[1].members, [a]);
        bind_table(&mut state, &note("two"), &[rows("read", "читать")], 0, mapping(), "en", "English", "b").unwrap();
        bind_table(&mut state, &note("three"), &[rows("write", "писать")], 0, mapping(), "en", "English", "c").unwrap();
        assert_eq!(state.registry.tables.len(), 3);
        assert_eq!(state.anchors.len(), 3);
    }

    #[test]
    fn edited_or_duplicated_table_cannot_steal_old_identity() {
        let n = note("one");
        let mut state = StudyState::default();
        bind_table(&mut state, &n, &[rows("run", "бегать")], 0, mapping(), "one", "One", "a").unwrap();
        let prior = state.clone();
        assert_eq!(
            bind_table(&mut state, &n, &[rows("run", "управлять")], 0, mapping(), "one", "One", "b"),
            Err(StateError::NeedsRebind)
        );
        assert_eq!(state, prior);
        assert_eq!(
            bind_table(&mut state, &n, &[rows("run", "бегать"), rows("run", "бегать")], 0, mapping(), "one", "One", "b"),
            Err(StateError::DuplicateTable)
        );
    }

    #[test]
    fn mapping_conflict_is_not_silently_rewritten_for_other_groups() {
        let n = note("one");
        let mut state = StudyState::default();
        let table = [rows("run", "бегать")];
        bind_table(&mut state, &n, &table, 0, mapping(), "en", "English", "a").unwrap();
        let before = state.clone();
        let reversed = ColumnMapping { question: 1, answers: vec![0], context: None };
        assert_eq!(
            bind_table(&mut state, &n, &table, 0, reversed, "rev", "Reverse", "b"),
            Err(StateError::MappingConflict)
        );
        assert_eq!(state, before);
    }

    #[test]
    fn stale_writer_is_not_permitted_to_commit() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("study.sqlite");
        let mut one = StudyStore::open(&db).unwrap();
        let mut stale = StudyStore::open(&db).unwrap();
        let n = note("one");
        one.bind(0, &n, &[rows("eat", "есть")], 0, mapping(), "en", "English", "one").unwrap();
        assert_eq!(
            stale.bind(0, &n, &[rows("read", "читать")], 0, mapping(), "en", "English", "two"),
            Err(StateError::RevisionConflict)
        );
    }

    #[test]
    fn foreign_database_is_not_adopted_or_reset() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("foreign.sqlite");
        let connection = Connection::open(&db).unwrap();
        connection.execute_batch("CREATE TABLE prior(id INTEGER);").unwrap();
        drop(connection);
        assert!(matches!(StudyStore::open(&db), Err(StateError::UnsupportedVersion)));
    }

    #[test]
    fn corrupt_payload_cannot_be_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("study.sqlite");
        let mut store = StudyStore::open(&db).unwrap();
        store.connection.execute("UPDATE language_study_state SET payload=?1 WHERE id=1",
            params![&b"{ bad"[..]]).unwrap();
        assert_eq!(store.load(), Err(StateError::Corrupt));
        assert_eq!(
            store.bind(0, &note("one"), &[rows("a", "b")], 0, mapping(), "en", "English", "id"),
            Err(StateError::Corrupt)
        );
    }

    #[test]
    fn creating_new_table_when_older_one_is_held_must_wait_for_review() {
        let n = note("one");
        let mut state = StudyState::default();
        bind_table(&mut state, &n, &[rows("a", "A")], 0, mapping(), "c", "C", "id").unwrap();
        assert_eq!(
            bind_table(&mut state, &n, &[rows("b", "B"), rows("a", "edited")], 0, mapping(), "c", "C", "new"),
            Err(StateError::NeedsRebind)
        );
    }
}
