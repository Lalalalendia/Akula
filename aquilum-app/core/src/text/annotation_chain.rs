//! LANG-03i: bounded reattachment through a host-verified sequence of edits.
//!
//! A digest or repeated word is never a stable document identity. The host
//! must supply a contiguous series of trusted editor transitions, admitted
//! under the same File Gate / DocumentHub scope and revision chain.
//! This pure Core projection validates the snapshots and deltas but cannot
//! authenticate their provenance or persist sidecar data on its own.
//!
//! If any intermediate edit touches the highlighted word, changes a
//! snapshot unexpectedly, or loses a transition, HOLD the entire chain.
//! Never search for a matching word at a new position.

use super::annotations::{
    reattach, AnchorError, DocumentIdentity, EditorDelta, HoldReason,
    Reattachment, TextAnchor,
};
use std::ops::Range;

/// Deliberate cap: long histories need a fresh trusted snapshot and manual
/// reconciliation rather than unbounded replay during an editor redraw.
pub const MAX_VERIFIED_EDIT_STEPS: usize = 32;

/// One editor transition (old snapshot, new snapshot, exact replaced span).
/// Both snapshots must come from the host's previously admitted editor,
/// never from a scan or guess based on a saved annotation.
pub struct VerifiedEditStep<'a> {
    pub before: &'a str,
    pub after: &'a str,
    pub replaced: Range<usize>,
    pub inserted: &'a str,
}

/// Rebind one old anchor to the exact final snapshot through a bounded
/// consecutive editor change history. This is an in-memory calculation only.
///
/// The caller must authenticate vault, note and editor revision for each
/// transition, and use a separate atomic compare-and-swap before updating
/// any persisted anchor. Successful validation is not a filesystem grant.
pub fn reattach_verified_chain(
    saved: &TextAnchor,
    document: &DocumentIdentity,
    initial: &str,
    final_snapshot: &str,
    steps: &[VerifiedEditStep<'_>],
) -> Result<Reattachment, AnchorError> {
    // Always authenticate the *starting snapshot*, even with zero changes.
    // This also validates IDs, scopes, byte boundaries and document limits.
    let mut current_anchor = match reattach(saved, document, initial, initial, None)? {
        Reattachment::Attached(anchor) => anchor,
        Reattachment::Hold(reason) => return Ok(Reattachment::Hold(reason)),
    };

    if steps.len() > MAX_VERIFIED_EDIT_STEPS {
        return Ok(Reattachment::Hold(HoldReason::InvalidEdit));
    }
    let mut current_text = initial;
    for step in steps {
        // Broken history cannot be repaired by looking for identical words.
        if step.before != current_text || step.after == step.before {
            return Ok(Reattachment::Hold(HoldReason::InvalidEdit));
        }
        let next = reattach(
            &current_anchor,
            document,
            step.before,
            step.after,
            Some(EditorDelta {
                replaced: step.replaced.clone(),
                inserted: step.inserted,
            }),
        )?;
        current_anchor = match next {
            Reattachment::Attached(anchor) => anchor,
            Reattachment::Hold(reason) => return Ok(Reattachment::Hold(reason)),
        };
        current_text = step.after;
    }

    if current_text != final_snapshot {
        return Ok(Reattachment::Hold(if steps.is_empty() {
            HoldReason::NoVerifiedEdit
        } else {
            HoldReason::InvalidEdit
        }));
    }
    Ok(Reattachment::Attached(current_anchor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::annotations::capture;

    fn scope() -> DocumentIdentity {
        DocumentIdentity { vault_id: "vault-1".into(), note_id: "note-42".into() }
    }

    fn first(text: &str, phrase: &str) -> TextAnchor {
        let offset = text.find(phrase).unwrap();
        capture(&scope(), "highlight-1", text, offset..offset + phrase.len()).unwrap()
    }

    fn confirmed(result: Reattachment) -> TextAnchor {
        match result {
            Reattachment::Attached(anchor) => anchor,
            Reattachment::Hold(reason) => panic!("unexpected HOLD: {reason:?}"),
        }
    }

    #[test]
    fn unicode_insert_and_delete_before_selection_chain_exactly() {
        let original = "run run";
        let after_one = "Сначала run run";
        let after_two = "Сначала run! run";
        let after_three = "run! run";
        let saved = first(original, "run");
        let steps = [
            VerifiedEditStep {
                before: original, after: after_one,
                replaced: 0..0, inserted: "Сначала ",
            },
            VerifiedEditStep {
                before: after_one, after: after_two,
                replaced: "Сначала run".len().."Сначала run".len(),
                inserted: "!",
            },
            VerifiedEditStep {
                before: after_two, after: after_three,
                replaced: 0.."Сначала ".len(), inserted: "",
            },
        ];
        let moved = confirmed(reattach_verified_chain(
            &saved, &scope(), original, after_three, &steps,
        ).unwrap());
        assert_eq!((moved.byte_start, moved.byte_end), (0, 3));
        assert_eq!(&after_three[moved.byte_start..moved.byte_end], "run");
        assert_ne!(saved.document_digest, moved.document_digest);
    }

    #[test]
    fn changes_after_selection_leave_byte_offsets_intact() {
        let old = "eat";
        let a = "eat —";
        let b = "eat — ел";
        let saved = first(old, "eat");
        let steps = [
            VerifiedEditStep { before: old, after: a, replaced: 3..3, inserted: " —" },
            VerifiedEditStep { before: a, after: b, replaced: a.len()..a.len(), inserted: " ел" },
        ];
        let next = confirmed(reattach_verified_chain(
            &saved, &scope(), old, b, &steps,
        ).unwrap());
        assert_eq!((next.byte_start, next.byte_end), (0, 3));
    }

    #[test]
    fn edited_selection_holds_even_when_duplicate_word_survives() {
        let old = "run run";
        let a = "x run run";
        let b = "x ran run";
        let saved = first(old, "run");
        let steps = [
            VerifiedEditStep { before: old, after: a, replaced: 0..0, inserted: "x " },
            VerifiedEditStep { before: a, after: b, replaced: 2..5, inserted: "ran" },
        ];
        assert_eq!(reattach_verified_chain(&saved, &scope(), old, b, &steps).unwrap(),
            Reattachment::Hold(HoldReason::SelectionEdited));
    }

    #[test]
    fn missing_middle_transition_cannot_reassign_to_a_duplicate() {
        let old = "run run";
        let a = "x run run";
        let b = "x run! run";
        let saved = first(old, "run");
        let steps = [VerifiedEditStep {
            before: a, after: b,
            replaced: 5..5, inserted: "!",
        }];
        assert_eq!(reattach_verified_chain(&saved, &scope(), old, b, &steps).unwrap(),
            Reattachment::Hold(HoldReason::InvalidEdit));
    }

    #[test]
    fn fabricated_delta_and_inconsistent_final_snapshot_hold() {
        let old = "eat ate";
        let a = "now eat ate";
        let saved = first(old, "eat");
        let wrong = [VerifiedEditStep {
            before: old, after: a, replaced: 0..0, inserted: "yesterday ",
        }];
        assert_eq!(reattach_verified_chain(&saved, &scope(), old, a, &wrong).unwrap(),
            Reattachment::Hold(HoldReason::InvalidEdit));
        let valid = [VerifiedEditStep {
            before: old, after: a, replaced: 0..0, inserted: "now ",
        }];
        assert_eq!(reattach_verified_chain(&saved, &scope(), old, "elsewhere", &valid).unwrap(),
            Reattachment::Hold(HoldReason::InvalidEdit));
    }

    #[test]
    fn no_history_only_allows_unchanged_original_snapshot() {
        let old = "Язык language";
        let saved = first(old, "Язык");
        let original = confirmed(reattach_verified_chain(
            &saved, &scope(), old, old, &[],
        ).unwrap());
        assert_eq!(original, saved);
        assert_eq!(reattach_verified_chain(
            &saved, &scope(), old, "другой текст", &[],
        ).unwrap(), Reattachment::Hold(HoldReason::NoVerifiedEdit));
    }

    #[test]
    fn oversized_history_and_unneeded_noop_edit_fail_closed() {
        let text = "run";
        let saved = first(text, "run");
        let oversized = (0..MAX_VERIFIED_EDIT_STEPS + 1)
            .map(|_| VerifiedEditStep {
                before: text, after: text, replaced: 0..0, inserted: "",
            })
            .collect::<Vec<_>>();
        assert_eq!(reattach_verified_chain(
            &saved, &scope(), text, text, &oversized,
        ).unwrap(), Reattachment::Hold(HoldReason::InvalidEdit));
        let noop = [VerifiedEditStep {
            before: text, after: text, replaced: 0..0, inserted: "",
        }];
        assert_eq!(reattach_verified_chain(
            &saved, &scope(), text, text, &noop,
        ).unwrap(), Reattachment::Hold(HoldReason::InvalidEdit));
    }

    #[test]
    fn foreign_vault_and_changed_initial_revision_are_never_admitted() {
        let old = "run";
        let saved = first(old, "run");
        let foreign = DocumentIdentity {
            vault_id: "vault-2".into(), note_id: scope().note_id,
        };
        assert_eq!(reattach_verified_chain(
            &saved, &foreign, old, old, &[],
        ), Err(AnchorError::ScopeMismatch));
        assert_eq!(reattach_verified_chain(
            &saved, &scope(), "ran", "ran", &[],
        ).unwrap(), Reattachment::Hold(HoldReason::OldSnapshotChanged));
    }
}
