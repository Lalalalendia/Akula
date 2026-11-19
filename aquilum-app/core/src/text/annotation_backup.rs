//! LANG-03l: versioned, document-scoped annotation metadata backup.
//!
//! The caller must authorize the active registered document and explicitly
//! choose an export/import destination. This Core module neither opens vault
//! files nor exports source Markdown/selected source snippets. Its opaque
//! document IDs are NOT read grants. Every imported anchor is set to HOLD
//! until a later trusted source-revision check/manual rebind admits it.
//!
//! The import is intentionally non-destructive: it refuses to replace any
//! existing metadata for the target document, and uses the existing SQLite
//! revision CAS so another writer cannot be silently overwritten.

use serde::{Deserialize, Serialize};
use super::annotation_store::{
    validate, AnnotationRecord, AnnotationStore, BindingStatus, Snapshot, StoreError,
};
use super::annotations::DocumentIdentity;

const BACKUP_FORMAT: &str = "akula-text-annotations";
const BACKUP_VERSION: u32 = 1;
const MAX_BACKUP_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackupError {
    InvalidIdentity,
    InvalidPayload,
    UnsupportedVersion,
    ForeignDocument,
    TooLarge,
    AlreadyPresent,
    RevisionConflict,
    Store(StoreError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportResult {
    pub revision: u64,
    pub imported: usize,
    pub held: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupEnvelope {
    format: String,
    version: u32,
    document: DocumentIdentity,
    records: Vec<AnnotationRecord>,
}

fn valid_id(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 128 && !s.chars().any(char::is_control)
}

fn valid_document(doc: &DocumentIdentity) -> bool {
    valid_id(&doc.vault_id) && valid_id(&doc.note_id)
}

/// Export only one explicitly authorized document, without raw source text.
/// Source authorization and choosing the destination belong to the native host.
pub fn export_document(
    snapshot: &Snapshot,
    authorized: &DocumentIdentity,
) -> Result<Vec<u8>, BackupError> {
    if !valid_document(authorized) {
        return Err(BackupError::InvalidIdentity);
    }
    validate(&snapshot.records).map_err(BackupError::Store)?;
    let records = snapshot.records.iter()
        .filter(|row| &row.anchor.document == authorized)
        .cloned().collect::<Vec<_>>();
    let bytes = serde_json::to_vec(&BackupEnvelope {
        format: BACKUP_FORMAT.to_owned(),
        version: BACKUP_VERSION,
        document: authorized.clone(),
        records,
    }).map_err(|_| BackupError::InvalidPayload)?;
    if bytes.len() > MAX_BACKUP_BYTES {
        return Err(BackupError::TooLarge);
    }
    Ok(bytes)
}

fn decode_backup(
    bytes: &[u8],
    authorized: &DocumentIdentity,
) -> Result<Vec<AnnotationRecord>, BackupError> {
    if !valid_document(authorized) {
        return Err(BackupError::InvalidIdentity);
    }
    if bytes.len() > MAX_BACKUP_BYTES {
        return Err(BackupError::TooLarge);
    }
    let envelope: BackupEnvelope =
        serde_json::from_slice(bytes).map_err(|_| BackupError::InvalidPayload)?;
    if envelope.format != BACKUP_FORMAT || envelope.version != BACKUP_VERSION {
        return Err(BackupError::UnsupportedVersion);
    }
    if envelope.document != *authorized {
        return Err(BackupError::ForeignDocument);
    }
    if envelope.records.iter().any(|row| row.anchor.document != *authorized) {
        return Err(BackupError::ForeignDocument);
    }
    validate(&envelope.records).map_err(BackupError::Store)?;
    // Portable byte offsets cannot prove the current document revision.
    // Imported records are never paintable until separately re-verified.
    let mut records = envelope.records;
    for record in &mut records {
        record.status = BindingStatus::NeedsRebind;
    }
    Ok(records)
}

/// Restore one document into a sidecar that has no annotations for that
/// document yet. Refuse to merge into existing user-authored metadata, even
/// if the imported annotation IDs happen to be distinct.
///
/// Sidecar access must still be granted by the host, not by these backup IDs.
pub fn import_document(
    store: &mut AnnotationStore,
    expected_revision: u64,
    authorized: &DocumentIdentity,
    bytes: &[u8],
) -> Result<ImportResult, BackupError> {
    let incoming = decode_backup(bytes, authorized)?;
    let snapshot = store.load().map_err(BackupError::Store)?;
    if snapshot.revision != expected_revision {
        return Err(BackupError::RevisionConflict);
    }
    if snapshot.records.iter().any(|row| row.anchor.document == *authorized) {
        return Err(BackupError::AlreadyPresent);
    }
    let count = incoming.len();
    if count == 0 {
        return Ok(ImportResult {
            revision: expected_revision, imported: 0, held: 0,
        });
    }
    let mut combined = snapshot.records;
    combined.extend(incoming);
    validate(&combined).map_err(BackupError::Store)?;
    let revision = store.replace(expected_revision, &combined).map_err(|error| {
        match error {
            StoreError::RevisionConflict => BackupError::RevisionConflict,
            other => BackupError::Store(other),
        }
    })?;
    Ok(ImportResult {
        revision, imported: count, held: count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::annotations::capture;
    use super::super::annotation_store::{PaletteColor, SourceReference};

    fn doc(id: &str) -> DocumentIdentity {
        DocumentIdentity {
            vault_id: "vault-1".into(), note_id: id.into(),
        }
    }

    fn row(d: &DocumentIdentity, id: &str) -> AnnotationRecord {
        AnnotationRecord {
            anchor: capture(d, id, "eat run", 0..3).unwrap(),
            color: PaletteColor::Blue,
            category: Some("Verb".into()),
            explanation: Some("Present tense".into()),
            reference: Some(SourceReference {
                namespace: "learning-row".into(),
                stable_id: "stable-row".into(),
            }),
            status: BindingStatus::Current,
        }
    }

    fn store() -> (tempfile::TempDir, AnnotationStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = AnnotationStore::open(&dir.path().join("annotations.sqlite")).unwrap();
        (dir, store)
    }

    #[test]
    fn scoped_export_never_copies_markdown_or_another_notes_metadata() {
        let a = doc("a"), b = doc("b");
        let one = row(&a, "annotation-a");
        let two = row(&b, "annotation-b");
        let backup = export_document(&Snapshot {
            revision: 9, records: vec![one, two],
        }, &a).unwrap();
        let str_backup = String::from_utf8(backup.clone()).unwrap();
        assert!(!str_backup.contains("eat run"));
        assert!(!str_backup.contains("annotation-b"));
        assert!(!str_backup.contains("\"note_id\":\"b\""));
        assert!(str_backup.contains("Present tense"));
        let decoded: BackupEnvelope = serde_json::from_slice(&backup).unwrap();
        assert_eq!(decoded.document, a);
        assert_eq!(decoded.records.len(), 1);
    }

    #[test]
    fn restores_metadata_as_hold_and_preserves_unrelated_documents() {
        let a = doc("a"), b = doc("b");
        let backup = export_document(&Snapshot {
            revision: 1, records: vec![row(&a, "annotation-a")],
        }, &a).unwrap();
        let (_dir, mut s) = store();
        let other = row(&b, "annotation-b");
        s.replace(0, &[other.clone()]).unwrap();
        let restored = import_document(&mut s, 1, &a, &backup).unwrap();
        assert_eq!(restored, ImportResult {
            revision: 2, imported: 1, held: 1,
        });
        let state = s.load().unwrap();
        assert_eq!(state.records[0], other);
        assert_eq!(state.records[1].status, BindingStatus::NeedsRebind);
        assert_eq!(state.records[1].explanation.as_deref(), Some("Present tense"));
        assert_eq!(state.records[1].reference.as_ref().unwrap().stable_id, "stable-row");
    }

    #[test]
    fn rejects_foreign_document_and_mixed_vault_payloads_without_writing() {
        let a = doc("a"), b = doc("b");
        let original = row(&a, "annotation-a");
        let backup = export_document(&Snapshot {
            revision: 1, records: vec![original.clone()],
        }, &a).unwrap();
        let (_dir, mut store) = store();
        assert_eq!(import_document(&mut store, 0, &b, &backup),
            Err(BackupError::ForeignDocument));
        let altered = BackupEnvelope {
            format: BACKUP_FORMAT.into(),
            version: BACKUP_VERSION,
            document: a.clone(),
            records: vec![row(&b, "foreign")],
        };
        let bytes = serde_json::to_vec(&altered).unwrap();
        assert_eq!(import_document(&mut store, 0, &a, &bytes),
            Err(BackupError::ForeignDocument));
        assert_eq!(store.load().unwrap().revision, 0);
    }

    #[test]
    fn prevents_existing_data_overwrite_and_races_with_revision_cas() {
        let a = doc("a");
        let backup = export_document(&Snapshot {
            revision: 1, records: vec![row(&a, "old")],
        }, &a).unwrap();
        let (_dir, mut store) = store();
        let original = row(&a, "current");
        store.replace(0, &[original.clone()]).unwrap();
        assert_eq!(import_document(&mut store, 1, &a, &backup),
            Err(BackupError::AlreadyPresent));
        assert_eq!(import_document(&mut store, 0, &a, &backup),
            Err(BackupError::RevisionConflict));
        assert_eq!(store.load().unwrap().records, vec![original]);
    }

    #[test]
    fn corrupt_future_oversize_and_empty_imports_are_non_destructive() {
        let a = doc("a");
        let (_dir, mut store) = store();
        assert_eq!(import_document(&mut store, 0, &a, b"{"),
            Err(BackupError::InvalidPayload));
        let future = serde_json::to_vec(&BackupEnvelope {
            format: BACKUP_FORMAT.into(),
            version: BACKUP_VERSION + 1,
            document: a.clone(),
            records: vec![],
        }).unwrap();
        assert_eq!(import_document(&mut store, 0, &a, &future),
            Err(BackupError::UnsupportedVersion));
        assert_eq!(import_document(&mut store, 0, &a, &vec![b'x'; MAX_BACKUP_BYTES + 1]),
            Err(BackupError::TooLarge));
        let empty = export_document(&Snapshot {
            revision: 0, records: vec![],
        }, &a).unwrap();
        assert_eq!(import_document(&mut store, 0, &a, &empty),
            Ok(ImportResult { revision: 0, imported: 0, held: 0 }));
        assert_eq!(store.load().unwrap().revision, 0);
    }
}
