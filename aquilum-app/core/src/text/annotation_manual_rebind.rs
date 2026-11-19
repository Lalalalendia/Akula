//! LANG-03o: explicitly authorized manual reattachment of a HOLD highlight.
//!
//! The native host must first admit the exact active, clean, registered note,
//! its current saved Markdown bytes, and a user-selected UTF-8 range. An
//! annotation ID or backup file never grants document access. This function
//! does not scan for similar text, guess locations, read Markdown, or write it.
//!
//! For now, only a unique held annotation with exactly the same selected text
//! digest can be reattached. A changed word or two indistinguishable held
//! records must remain on HOLD until a more explicit selection UX exists.

use std::ops::Range;

use super::annotation_store::{AnnotationStore, BindingStatus, StoreError};
use super::annotations::{capture, AnchorError, DocumentIdentity};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManualRebindError {
    Store(StoreError),
    Anchor(AnchorError),
    RevisionConflict,
    NoMatchingHeld,
    AmbiguousHeld,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebindResult {
    pub revision: u64,
    pub annotation_id: String,
}

/// The caller must prove the source document is user-admitted, clean and
/// still identical to the editor snapshot when performing this SQLite CAS.
/// Only the *user's explicit selection* supplies the new coordinate.
pub fn bind_explicit_selection(
    store: &mut AnnotationStore,
    expected_revision: u64,
    authorized: &DocumentIdentity,
    current_markdown: &str,
    selected: Range<usize>,
) -> Result<RebindResult, ManualRebindError> {
    let candidate = capture(authorized, "manual-selection", current_markdown, selected)
        .map_err(ManualRebindError::Anchor)?;
    let mut snapshot = store.load().map_err(ManualRebindError::Store)?;
    if snapshot.revision != expected_revision {
        return Err(ManualRebindError::RevisionConflict);
    }

    let mut matches = snapshot.records.iter().enumerate().filter(|(_, row)| {
        row.status == BindingStatus::NeedsRebind
            && &row.anchor.document == authorized
            && row.anchor.selected_digest == candidate.selected_digest
            && row.anchor.byte_end - row.anchor.byte_start
                == candidate.byte_end - candidate.byte_start
    });
    let Some((index, _)) = matches.next() else {
        return Err(ManualRebindError::NoMatchingHeld);
    };
    if matches.next().is_some() {
        return Err(ManualRebindError::AmbiguousHeld);
    }

    let row = &mut snapshot.records[index];
    let annotation_id = row.anchor.annotation_id.clone();
    row.anchor.byte_start = candidate.byte_start;
    row.anchor.byte_end = candidate.byte_end;
    row.anchor.selected_digest = candidate.selected_digest;
    row.anchor.document_digest = candidate.document_digest;
    row.status = BindingStatus::Current;

    let revision = store.replace(expected_revision, &snapshot.records)
        .map_err(|error| match error {
            StoreError::RevisionConflict => ManualRebindError::RevisionConflict,
            other => ManualRebindError::Store(other),
        })?;
    Ok(RebindResult { revision, annotation_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::annotation_backup::{export_document, import_document};
    use super::super::annotation_store::{
        AnnotationRecord, PaletteColor, SourceReference, Snapshot,
    };
    use super::super::annotations::capture;

    fn doc(name: &str) -> DocumentIdentity {
        DocumentIdentity { vault_id: "vault-A".into(), note_id: name.into() }
    }

    fn row(d: &DocumentIdentity, id: &str, text: &str, at: usize) -> AnnotationRecord {
        AnnotationRecord {
            anchor: capture(d, id, text, at..at + 3).unwrap(),
            color: PaletteColor::Blue,
            category: Some("verbs".into()),
            explanation: Some("User-authored note".into()),
            reference: Some(SourceReference {
                namespace: "study-row".into(), stable_id: "row-1".into(),
            }),
            status: BindingStatus::NeedsRebind,
        }
    }

    fn store() -> (tempfile::TempDir, AnnotationStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = AnnotationStore::open(&dir.path().join("annotations.sqlite")).unwrap();
        (dir, store)
    }

    #[test]
    fn imported_held_backup_can_rebind_only_explicit_exact_unicode_selection() {
        let a = doc("a");
        let original = row(&a, "highlight-A", "eat run", 0);
        let bytes = export_document(&Snapshot {
            revision: 0, records: vec![original.clone()],
        }, &a).unwrap();
        let (_dir, mut s) = store();
        let report = import_document(&mut s, 0, &a, &bytes).unwrap();
        assert_eq!(report.held, 1);
        let text = "✦ eat run";
        let start = text.find("eat").unwrap();
        let result = bind_explicit_selection(
            &mut s, report.revision, &a, text, start..start + 3,
        ).unwrap();
        assert_eq!(result.annotation_id, "highlight-A");
        let saved = s.load().unwrap();
        assert_eq!(saved.revision, result.revision);
        assert_eq!(saved.records[0].status, BindingStatus::Current);
        assert_eq!((saved.records[0].anchor.byte_start, saved.records[0].anchor.byte_end),
            (start, start + 3));
        assert_eq!(saved.records[0].color, original.color);
        assert_eq!(saved.records[0].explanation, original.explanation);
        assert_eq!(saved.records[0].reference, original.reference);
        assert_ne!(saved.records[0].anchor.document_digest, original.anchor.document_digest);
    }

    #[test]
    fn same_phrase_in_two_held_records_is_ambiguous_and_never_written() {
        let a = doc("a");
        let old = "eat eat";
        let original = vec![row(&a, "one", old, 0), row(&a, "two", old, 4)];
        let (_dir, mut s) = store();
        s.replace(0, &original).unwrap();
        assert_eq!(bind_explicit_selection(&mut s, 1, &a, old, 0..3),
            Err(ManualRebindError::AmbiguousHeld));
        assert_eq!(s.load().unwrap().revision, 1);
        assert_eq!(s.load().unwrap().records, original);
    }

    #[test]
    fn changed_selected_text_and_invalid_ranges_are_refused_without_write() {
        let a = doc("a");
        let old = row(&a, "one", "eat run", 0);
        let (_dir, mut s) = store();
        s.replace(0, &[old.clone()]).unwrap();
        let text = "ran eat";
        assert_eq!(bind_explicit_selection(&mut s, 1, &a, text, 0..3),
            Err(ManualRebindError::NoMatchingHeld));
        assert_eq!(bind_explicit_selection(&mut s, 1, &a, text, 2..2),
            Err(ManualRebindError::Anchor(AnchorError::InvalidRange)));
        assert_eq!(s.load().unwrap().revision, 1);
        assert_eq!(s.load().unwrap().records, vec![old]);
    }

    #[test]
    fn forged_scope_and_stale_sqlite_revision_fail_closed() {
        let a = doc("a");
        let b = doc("b");
        let old = row(&a, "one", "eat run", 0);
        let (_dir, mut s) = store();
        s.replace(0, &[old.clone()]).unwrap();
        assert_eq!(bind_explicit_selection(&mut s, 1, &b, "eat run", 0..3),
            Err(ManualRebindError::NoMatchingHeld));
        assert_eq!(bind_explicit_selection(&mut s, 0, &a, "eat run", 0..3),
            Err(ManualRebindError::RevisionConflict));
        assert_eq!(s.load().unwrap().records, vec![old]);
    }

    #[test]
    fn current_spans_and_unrelated_documents_are_never_retargeted() {
        let a = doc("a");
        let b = doc("b");
        let original = row(&a, "held", "eat run", 0);
        let foreign = row(&b, "foreign", "eat run", 0);
        let mut current = row(&a, "current", "run eat", 4);
        current.status = BindingStatus::Current;
        let (_dir, mut s) = store();
        s.replace(0, &[original.clone(), foreign.clone(), current.clone()]).unwrap();
        assert_eq!(bind_explicit_selection(&mut s, 1, &a, "eat run", 0..3).unwrap().revision, 2);
        let rows = s.load().unwrap().records;
        assert_eq!(rows[0].status, BindingStatus::Current);
        assert_eq!(rows[1], foreign);
        assert_eq!(rows[2], current);
    }
}
