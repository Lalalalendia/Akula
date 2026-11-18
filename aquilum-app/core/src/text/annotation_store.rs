//! LANG-03b: bounded local sidecar for manual annotation metadata.
//!
//! The host supplies an app-data database path and must authorize the note,
//! capture a fresh editor snapshot, and enforce DocumentHub revision checks.
//! This module never opens a vault path or writes source Markdown. Selected
//! source text is NOT persisted: only its opaque identity and digest are.
//! User-authored labels and explanations are intentionally persisted.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::annotations::{DocumentIdentity, TextAnchor, MAX_ANNOTATED_SPAN_BYTES, MAX_DOCUMENT_BYTES};

const VERSION: i64 = 1;
const MAX_RECORDS: usize = 4096;
const MAX_PER_DOCUMENT: usize = 512;
const MAX_PAYLOAD_BYTES: usize = 4 * 1024 * 1024;
const MAX_EXPLANATION_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaletteColor {
    Yellow,
    Green,
    Violet,
    Blue,
    Orange,
    Pink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingStatus {
    Current,
    NeedsRebind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceReference {
    /// Namespace and ID are opaque locators, never reading grants.
    pub namespace: String,
    pub stable_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnnotationRecord {
    pub anchor: TextAnchor,
    pub color: PaletteColor,
    pub category: Option<String>,
    pub explanation: Option<String>,
    pub reference: Option<SourceReference>,
    pub status: BindingStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    Storage,
    Corrupt,
    UnsupportedVersion,
    InvalidRecord,
    DuplicateIdentity,
    TooManyRecords,
    TooLarge,
    RevisionConflict,
    RevisionExhausted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub revision: u64,
    pub records: Vec<AnnotationRecord>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    version: u32,
    records: Vec<AnnotationRecord>,
}

fn valid_id(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128
        && !value.chars().any(char::is_control)
}

fn valid_label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 96
        && !value.chars().any(char::is_control)
}

fn valid_explanation(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= MAX_EXPLANATION_BYTES
        && !value.chars().any(|c| c == '\0')
}

/// Validate sidecar content before saving or returning it to the host.
/// The host is still responsible for admission and editor revision checks.
pub fn validate(records: &[AnnotationRecord]) -> Result<(), StoreError> {
    if records.len() > MAX_RECORDS {
        return Err(StoreError::TooManyRecords);
    }
    let mut ids = BTreeSet::new();
    let mut per_doc = BTreeMap::<(&str, &str), usize>::new();
    for record in records {
        let anchor = &record.anchor;
        let doc = &anchor.document;
        if !valid_id(&doc.vault_id) || !valid_id(&doc.note_id)
            || !valid_id(&anchor.annotation_id)
            || anchor.byte_end <= anchor.byte_start
            || anchor.byte_end > MAX_DOCUMENT_BYTES
            || anchor.byte_end - anchor.byte_start > MAX_ANNOTATED_SPAN_BYTES
            || record.category.as_deref().is_some_and(|v| !valid_label(v))
            || record.explanation.as_deref().is_some_and(|v| !valid_explanation(v))
            || record.reference.as_ref().is_some_and(|r|
                !valid_id(&r.namespace) || !valid_id(&r.stable_id))
        {
            return Err(StoreError::InvalidRecord);
        }
        let key = (doc.vault_id.as_str(), doc.note_id.as_str());
        if !ids.insert((key.0, key.1, anchor.annotation_id.as_str())) {
            return Err(StoreError::DuplicateIdentity);
        }
        let count = per_doc.entry(key).or_default();
        *count += 1;
        if *count > MAX_PER_DOCUMENT {
            return Err(StoreError::TooManyRecords);
        }
    }
    Ok(())
}

fn encode(records: &[AnnotationRecord]) -> Result<Vec<u8>, StoreError> {
    validate(records)?;
    let bytes = serde_json::to_vec(&Payload { version: VERSION as u32, records: records.to_vec() })
        .map_err(|_| StoreError::Storage)?;
    if bytes.len() > MAX_PAYLOAD_BYTES {
        return Err(StoreError::TooLarge);
    }
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Result<Vec<AnnotationRecord>, StoreError> {
    if bytes.len() > MAX_PAYLOAD_BYTES {
        return Err(StoreError::Corrupt);
    }
    let payload: Payload = serde_json::from_slice(bytes).map_err(|_| StoreError::Corrupt)?;
    if i64::from(payload.version) != VERSION {
        return Err(StoreError::UnsupportedVersion);
    }
    validate(&payload.records).map_err(|_| StoreError::Corrupt)?;
    Ok(payload.records)
}

pub struct AnnotationStore {
    connection: Connection,
}

impl AnnotationStore {
    /// Must receive an app-owned data path, never a user note or vault path.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let mut connection = Connection::open(path).map_err(|_| StoreError::Storage)?;
        // Reject an unrelated database before changing its journal mode.
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|_| StoreError::Storage)?;
        if version == 0 {
            let existing: i64 = connection.query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                [], |row| row.get(0),
            ).map_err(|_| StoreError::Storage)?;
            if existing != 0 {
                return Err(StoreError::UnsupportedVersion);
            }
        } else if version != VERSION {
            return Err(StoreError::UnsupportedVersion);
        }
        connection.execute_batch(
            "PRAGMA foreign_keys=ON;
             PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA busy_timeout=3000;",
        ).map_err(|_| StoreError::Storage)?;
        if version == 0 {
            let initial = encode(&[])?;
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| StoreError::Storage)?;
            tx.execute_batch(
                "CREATE TABLE text_annotation_state (
                   id INTEGER PRIMARY KEY CHECK(id=1),
                   revision INTEGER NOT NULL CHECK(revision>=0),
                   payload BLOB NOT NULL
                 );",
            ).map_err(|_| StoreError::Storage)?;
            tx.execute(
                "INSERT INTO text_annotation_state(id,revision,payload) VALUES(1,0,?1)",
                params![initial],
            ).map_err(|_| StoreError::Storage)?;
            tx.execute_batch("PRAGMA user_version=1;")
                .map_err(|_| StoreError::Storage)?;
            tx.commit().map_err(|_| StoreError::Storage)?;
        }
        let store = Self { connection };
        store.load()?;
        Ok(store)
    }

    pub fn load(&self) -> Result<Snapshot, StoreError> {
        let (rev, bytes): (i64, Vec<u8>) = self.connection.query_row(
            "SELECT revision,payload FROM text_annotation_state WHERE id=1",
            [], |r| Ok((r.get(0)?, r.get(1)?)),
        ).map_err(|_| StoreError::Corrupt)?;
        Ok(Snapshot {
            revision: u64::try_from(rev).map_err(|_| StoreError::Corrupt)?,
            records: decode(&bytes)?,
        })
    }

    /// Atomically replace one complete metadata snapshot only after a CAS.
    /// The host must check note grants and editor provenance before calling.
    pub fn replace(
        &mut self, expected_revision: u64, records: &[AnnotationRecord],
    ) -> Result<u64, StoreError> {
        let updated = encode(records)?;
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| StoreError::Storage)?;
        let (rev, previous): (i64, Vec<u8>) = tx.query_row(
            "SELECT revision,payload FROM text_annotation_state WHERE id=1",
            [], |r| Ok((r.get(0)?, r.get(1)?)),
        ).map_err(|_| StoreError::Corrupt)?;
        let prior = decode(&previous)?;
        let expected = u64::try_from(rev).map_err(|_| StoreError::Corrupt)?;
        if expected != expected_revision {
            return Err(StoreError::RevisionConflict);
        }
        if prior.as_slice() == records {
            return Ok(expected);
        }
        let next = rev.checked_add(1).ok_or(StoreError::RevisionExhausted)?;
        let changed = tx.execute(
            "UPDATE text_annotation_state SET revision=?1,payload=?2
             WHERE id=1 AND revision=?3",
            params![next, updated, rev],
        ).map_err(|_| StoreError::Storage)?;
        if changed != 1 {
            return Err(StoreError::RevisionConflict);
        }
        tx.commit().map_err(|_| StoreError::Storage)?;
        u64::try_from(next).map_err(|_| StoreError::RevisionExhausted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::annotations::{capture, MAX_DOCUMENT_BYTES};

    fn doc() -> DocumentIdentity {
        DocumentIdentity { vault_id: "vault-A".into(), note_id: "doc-A".into() }
    }

    fn record() -> AnnotationRecord {
        AnnotationRecord {
            anchor: capture(&doc(), "annotation-1", "The courier ate pizza", 12..15).unwrap(),
            color: PaletteColor::Yellow,
            category: Some("Past Simple".into()),
            explanation: Some("eat → ate → eaten".into()),
            reference: Some(SourceReference {
                namespace: "learning-row".into(), stable_id: "opaque-row-id".into(),
            }),
            status: BindingStatus::Current,
        }
    }

    #[test]
    fn roundtrip_and_reopen_preserve_metadata_without_copying_source() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("annotations.sqlite");
        let mut store = AnnotationStore::open(&db).unwrap();
        assert_eq!(store.load().unwrap().revision, 0);
        let original = record();
        assert_eq!(store.replace(0, std::slice::from_ref(&original)), Ok(1));
        assert_eq!(store.replace(1, std::slice::from_ref(&original)), Ok(1));
        drop(store);
        let reopened = AnnotationStore::open(&db).unwrap();
        assert_eq!(reopened.load().unwrap(), Snapshot {
            revision: 1, records: vec![original],
        });
        let bytes = std::fs::read(&db).unwrap();
        assert!(!bytes.windows("The courier ate pizza".len())
            .any(|w| w == "The courier ate pizza".as_bytes()));
    }

    #[test]
    fn concurrent_stale_writer_cannot_override_updated_explanation() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("annotations.sqlite");
        let mut one = AnnotationStore::open(&db).unwrap();
        let mut stale = AnnotationStore::open(&db).unwrap();
        let mut rec = record();
        one.replace(0, &[rec.clone()]).unwrap();
        rec.explanation = Some("Updated manually".into());
        assert_eq!(stale.replace(0, &[rec]), Err(StoreError::RevisionConflict));
        assert_eq!(one.load().unwrap().revision, 1);
    }

    #[test]
    fn hold_status_and_color_changes_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("annotations.sqlite");
        let mut store = AnnotationStore::open(&db).unwrap();
        let mut rec = record();
        store.replace(0, &[rec.clone()]).unwrap();
        rec.status = BindingStatus::NeedsRebind;
        rec.color = PaletteColor::Violet;
        store.replace(1, &[rec.clone()]).unwrap();
        drop(store);
        let saved = AnnotationStore::open(&db).unwrap().load().unwrap();
        assert_eq!(saved.revision, 2);
        assert_eq!(saved.records, vec![rec]);
    }

    #[test]
    fn duplicate_ids_in_same_document_are_rejected_without_write() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("annotations.sqlite");
        let mut store = AnnotationStore::open(&db).unwrap();
        let rec = record();
        assert_eq!(store.replace(0, &[rec.clone(), rec]),
            Err(StoreError::DuplicateIdentity));
        assert_eq!(store.load().unwrap().revision, 0);
    }

    #[test]
    fn invalid_and_oversized_records_are_rejected() {
        let mut rec = record();
        rec.category = Some("x".repeat(97));
        assert_eq!(validate(&[rec.clone()]), Err(StoreError::InvalidRecord));
        rec.category = None;
        rec.explanation = Some("x".repeat(MAX_EXPLANATION_BYTES + 1));
        assert_eq!(validate(&[rec.clone()]), Err(StoreError::InvalidRecord));
        rec.explanation = None;
        rec.anchor.byte_end = MAX_DOCUMENT_BYTES + 1;
        assert_eq!(validate(&[rec]), Err(StoreError::InvalidRecord));
    }

    #[test]
    fn foreign_database_or_corrupt_payload_cannot_be_adopted_or_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("annotations.sqlite");
        let other = Connection::open(&db).unwrap();
        other.execute_batch("CREATE TABLE stranger(id INTEGER);").unwrap();
        drop(other);
        assert!(matches!(AnnotationStore::open(&db),
            Err(StoreError::UnsupportedVersion)));

        let db = dir.path().join("another.sqlite");
        let mut store = AnnotationStore::open(&db).unwrap();
        store.connection.execute(
            "UPDATE text_annotation_state SET payload=?1 WHERE id=1",
            params![&b"not json"[..]],
        ).unwrap();
        assert_eq!(store.load(), Err(StoreError::Corrupt));
        assert_eq!(store.replace(0, &[record()]), Err(StoreError::Corrupt));
    }

    #[test]
    fn two_authorized_documents_may_use_same_local_annotation_label() {
        let mut left = record();
        let mut right = record();
        right.anchor.document.note_id = "doc-B".into();
        left.reference = None;
        assert_eq!(validate(&[left, right]), Ok(()));
    }
}
