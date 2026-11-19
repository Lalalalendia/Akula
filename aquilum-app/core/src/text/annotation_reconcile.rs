//! LANG-03j: atomic metadata reconciliation after host-verified editor edits.
//!
//! This pure Core/sidecar operation cannot authenticate a File Gate grant,
//! editor provenance, canonical note path or disk revision by itself. The host
//! must do that before passing its contiguous trusted edit sequence.
//!
//! Stored anchors are not file permissions. No Markdown or selected source
//! text is copied into the sidecar. All changes use its revision CAS.

use super::annotation_chain::{reattach_verified_chain, VerifiedEditStep, MAX_VERIFIED_EDIT_STEPS};
use super::annotation_store::{
    AnnotationRecord, AnnotationStore, BindingStatus, Snapshot, StoreError,
};
use super::annotations::{
    AnchorError, DocumentIdentity, HoldReason, Reattachment, MAX_DOCUMENT_BYTES,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReconcileError {
    Store(StoreError),
    Anchor(AnchorError),
    InvalidHistory,
    RevisionConflict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconcileReport {
    pub revision: u64,
    pub reattached: usize,
    pub held: usize,
    pub untouched: usize,
}

/// Validate the *whole* sequence even if no current anchors match this note.
/// An untrusted/missing edit must never trigger a partial sidecar write.
fn validate_history(
    initial: &str,
    final_snapshot: &str,
    steps: &[VerifiedEditStep<'_>],
) -> Result<(), ReconcileError> {
    if initial.len() > MAX_DOCUMENT_BYTES
        || final_snapshot.len() > MAX_DOCUMENT_BYTES
        || steps.len() > MAX_VERIFIED_EDIT_STEPS
    {
        return Err(ReconcileError::InvalidHistory);
    }
    let mut cursor = initial;
    for step in steps {
        let start = step.replaced.start;
        let end = step.replaced.end;
        if step.before != cursor
            || step.after == step.before
            || step.after.len() > MAX_DOCUMENT_BYTES
            || start > end
            || end > cursor.len()
            || !cursor.is_char_boundary(start)
            || !cursor.is_char_boundary(end)
        {
            return Err(ReconcileError::InvalidHistory);
        }
        let Some(insert_end) = start.checked_add(step.inserted.len()) else {
            return Err(ReconcileError::InvalidHistory);
        };
        if step.after.get(..start) != cursor.get(..start)
            || step.after.get(start..insert_end) != Some(step.inserted)
            || step.after.get(insert_end..) != cursor.get(end..)
        {
            return Err(ReconcileError::InvalidHistory);
        }
        cursor = step.after;
    }
    if cursor != final_snapshot {
        return Err(ReconcileError::InvalidHistory);
    }
    Ok(())
}

/// Construct a proposed update without IO or mutable external state.
/// Existing held records are never guessed back into place.
pub fn plan_reconcile(
    snapshot: &Snapshot,
    document: &DocumentIdentity,
    initial: &str,
    final_snapshot: &str,
    steps: &[VerifiedEditStep<'_>],
) -> Result<(Vec<AnnotationRecord>, ReconcileReport), ReconcileError> {
    validate_history(initial, final_snapshot, steps)?;
    let mut records = snapshot.records.clone();
    let (mut reattached, mut held, mut untouched) = (0, 0, 0);
    for record in &mut records {
        if &record.anchor.document != document || record.status != BindingStatus::Current {
            untouched += 1;
            continue;
        }
        match reattach_verified_chain(
            &record.anchor, document, initial, final_snapshot, steps,
        ).map_err(ReconcileError::Anchor)? {
            Reattachment::Attached(anchor) => {
                if anchor != record.anchor {
                    record.anchor = anchor;
                    reattached += 1;
                } else {
                    untouched += 1;
                }
            }
            Reattachment::Hold(reason) => match reason {
                HoldReason::SelectionEdited
                | HoldReason::SelectionMismatch
                | HoldReason::OldSnapshotChanged => {
                    record.status = BindingStatus::NeedsRebind;
                    held += 1;
                }
                HoldReason::NoVerifiedEdit | HoldReason::InvalidEdit => {
                    return Err(ReconcileError::InvalidHistory);
                }
            },
        }
    }
    Ok((records, ReconcileReport {
        revision: snapshot.revision, reattached, held, untouched,
    }))
}

/// Persist the entire proposal with atomic revision CAS, or change nothing.
/// Caller must verify authorized editor identity and disk revision separately.
pub fn reconcile_sidecar(
    store: &mut AnnotationStore,
    expected_revision: u64,
    document: &DocumentIdentity,
    initial: &str,
    final_snapshot: &str,
    steps: &[VerifiedEditStep<'_>],
) -> Result<ReconcileReport, ReconcileError> {
    let snapshot = store.load().map_err(ReconcileError::Store)?;
    if snapshot.revision != expected_revision {
        return Err(ReconcileError::RevisionConflict);
    }
    let (records, mut report) =
        plan_reconcile(&snapshot, document, initial, final_snapshot, steps)?;
    report.revision = store.replace(expected_revision, &records)
        .map_err(|error| match error {
            StoreError::RevisionConflict => ReconcileError::RevisionConflict,
            other => ReconcileError::Store(other),
        })?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::annotation_store::PaletteColor;
    use super::super::annotations::capture;

    fn doc() -> DocumentIdentity {
        DocumentIdentity {
            vault_id: "vault-A".into(), note_id: "note-1".into(),
        }
    }

    fn other_doc() -> DocumentIdentity {
        DocumentIdentity {
            vault_id: "vault-A".into(), note_id: "note-2".into(),
        }
    }

    fn record(scope: &DocumentIdentity, id: &str, body: &str, start: usize) -> AnnotationRecord {
        AnnotationRecord {
            anchor: capture(scope, id, body, start..start + 3).unwrap(),
            color: PaletteColor::Violet,
            category: Some("Grammar".into()),
            explanation: Some("User authored note".into()),
            reference: None,
            status: BindingStatus::Current,
        }
    }

    fn store() -> (tempfile::TempDir, AnnotationStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = AnnotationStore::open(&dir.path().join("annotations.sqlite")).unwrap();
        (dir, store)
    }

    #[test]
    fn moves_two_duplicate_words_atomically_and_preserves_other_notes() {
        let (_dir, mut s) = store();
        let original = "run run";
        let foreign = record(&other_doc(), "c", original, 0);
        s.replace(0, &[
            record(&doc(), "a", original, 0),
            record(&doc(), "b", original, 4),
            foreign.clone(),
        ]).unwrap();
        let second = "x run run";
        let final_text = "x run run!";
        let edits = [
            VerifiedEditStep {
                before: original, after: second, replaced: 0..0, inserted: "x ",
            },
            VerifiedEditStep {
                before: second, after: final_text,
                replaced: second.len()..second.len(), inserted: "!",
            },
        ];
        let report = reconcile_sidecar(
            &mut s, 1, &doc(), original, final_text, &edits,
        ).unwrap();
        assert_eq!(report, ReconcileReport {
            revision: 2, reattached: 2, held: 0, untouched: 1,
        });
        let saved = s.load().unwrap();
        assert_eq!(saved.records[0].anchor.byte_start, 2);
        assert_eq!(saved.records[1].anchor.byte_start, 6);
        assert_eq!(saved.records[2], foreign);
        assert_eq!(saved.records[0].explanation.as_deref(), Some("User authored note"));
        assert_eq!(saved.records[0].category.as_deref(), Some("Grammar"));
    }

    #[test]
    fn edited_selection_holds_only_that_meaning_not_surviving_duplicate() {
        let (_dir, mut s) = store();
        let old = "run run";
        s.replace(0, &[
            record(&doc(), "a", old, 0), record(&doc(), "b", old, 4),
        ]).unwrap();
        let new = "ran run";
        let step = [VerifiedEditStep {
            before: old, after: new, replaced: 0..3, inserted: "ran",
        }];
        let r = reconcile_sidecar(&mut s, 1, &doc(), old, new, &step).unwrap();
        assert_eq!((r.reattached, r.held), (1, 1));
        let rows = s.load().unwrap().records;
        assert_eq!(rows[0].status, BindingStatus::NeedsRebind);
        assert_eq!(rows[1].status, BindingStatus::Current);
        assert_eq!(rows[0].anchor.byte_start, 0);
        assert_eq!(rows[1].anchor.byte_start, 4);
    }

    #[test]
    fn missing_or_forged_transition_never_writes() {
        let (_dir, mut s) = store();
        let original = record(&doc(), "a", "run run", 0);
        s.replace(0, &[original.clone()]).unwrap();
        assert_eq!(reconcile_sidecar(
            &mut s, 1, &doc(), "run run", "x run run", &[],
        ), Err(ReconcileError::InvalidHistory));
        let lie = [VerifiedEditStep {
            before: "run run", after: "x run run", replaced: 0..0, inserted: "wrong ",
        }];
        assert_eq!(reconcile_sidecar(
            &mut s, 1, &doc(), "run run", "x run run", &lie,
        ), Err(ReconcileError::InvalidHistory));
        let saved = s.load().unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(saved.records, vec![original]);
    }

    #[test]
    fn concurrent_sidecar_note_edit_protects_against_stale_cas() {
        let (_dir, mut s) = store();
        let mut rec = record(&doc(), "a", "run", 0);
        s.replace(0, &[rec.clone()]).unwrap();
        rec.explanation = Some("Changed by another editor".into());
        s.replace(1, &[rec.clone()]).unwrap();
        let step = [VerifiedEditStep {
            before: "run", after: "x run", replaced: 0..0, inserted: "x ",
        }];
        assert_eq!(reconcile_sidecar(
            &mut s, 1, &doc(), "run", "x run", &step,
        ), Err(ReconcileError::RevisionConflict));
        assert_eq!(s.load().unwrap().records, vec![rec]);
    }

    #[test]
    fn unmodified_document_does_not_advance_sidecar_revision() {
        let (_dir, mut s) = store();
        let original = record(&doc(), "a", "run", 0);
        s.replace(0, &[original.clone()]).unwrap();
        let report = reconcile_sidecar(&mut s, 1, &doc(), "run", "run", &[]).unwrap();
        assert_eq!(report, ReconcileReport {
            revision: 1, reattached: 0, held: 0, untouched: 1,
        });
        assert_eq!(s.load().unwrap().records, vec![original]);
    }

    #[test]
    fn stale_and_previously_held_records_remain_fail_closed() {
        let (_dir, mut s) = store();
        let old = "run run";
        let mut stale = record(&doc(), "stale", old, 0);
        stale.anchor.document_digest = [0; 32];
        let mut held = record(&doc(), "held", old, 4);
        held.status = BindingStatus::NeedsRebind;
        s.replace(0, &[stale, held.clone()]).unwrap();
        let next = "x run run";
        let chain = [VerifiedEditStep {
            before: old, after: next, replaced: 0..0, inserted: "x ",
        }];
        let result = reconcile_sidecar(&mut s, 1, &doc(), old, next, &chain).unwrap();
        assert_eq!((result.held, result.untouched), (1, 1));
        let after = s.load().unwrap().records;
        assert_eq!(after[0].status, BindingStatus::NeedsRebind);
        assert_eq!(after[1], held);
    }

    #[test]
    fn broken_utf8_boundaries_and_history_limit_are_rejected() {
        let initial = "слово";
        let state = Snapshot { revision: 0, records: vec![] };
        let invalid = [VerifiedEditStep {
            before: initial, after: "xслово", replaced: 1..1, inserted: "x",
        }];
        assert_eq!(plan_reconcile(
            &state, &doc(), initial, "xслово", &invalid,
        ), Err(ReconcileError::InvalidHistory));
        let huge = (0..MAX_VERIFIED_EDIT_STEPS + 1)
            .map(|_| VerifiedEditStep {
                before: initial, after: initial, replaced: 0..0, inserted: "",
            }).collect::<Vec<_>>();
        assert_eq!(plan_reconcile(
            &state, &doc(), initial, initial, &huge,
        ), Err(ReconcileError::InvalidHistory));
    }

    #[test]
    fn foreign_vault_records_are_not_touched() {
        let (_dir, mut s) = store();
        let foreign = DocumentIdentity {
            vault_id: "vault-B".into(), note_id: doc().note_id,
        };
        let foreign_row = record(&foreign, "foreign", "run", 0);
        s.replace(0, &[foreign_row.clone()]).unwrap();
        let steps = [VerifiedEditStep {
            before: "run", after: "x run", replaced: 0..0, inserted: "x ",
        }];
        let result = reconcile_sidecar(
            &mut s, 1, &doc(), "run", "x run", &steps,
        ).unwrap();
        assert_eq!(result, ReconcileReport {
            revision: 1, reattached: 0, held: 0, untouched: 1,
        });
        assert_eq!(s.load().unwrap().records, vec![foreign_row]);
    }
}
