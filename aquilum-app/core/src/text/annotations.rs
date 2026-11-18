//! LANG-03a: fail-closed text-span identity for optional editor annotations.
//!
//! Pure Core contract, not a vault reader, editor mutation or permission grant.
//! Host supplies authorized UTF-8 snapshots plus an exact verified edit delta.
//! Annotation content is never saved in the anchor: hashes are matching
//! evidence, never IDs. Without trusted edit provenance, changes remain HOLD.

use std::ops::Range;
use serde::{Deserialize, Serialize};

pub const MAX_DOCUMENT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_ANNOTATED_SPAN_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentIdentity {
    pub vault_id: String,
    pub note_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextAnchor {
    pub annotation_id: String,
    pub document: DocumentIdentity,
    /// Ephemeral byte offsets of one authorized text revision.
    pub byte_start: usize,
    pub byte_end: usize,
    pub selected_digest: [u8; 32],
    pub document_digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorError {
    InvalidIdentity,
    InvalidRange,
    TooMuchText,
    ScopeMismatch,
    CorruptAnchor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldReason {
    OldSnapshotChanged,
    NoVerifiedEdit,
    InvalidEdit,
    SelectionEdited,
    SelectionMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reattachment {
    Attached(TextAnchor),
    Hold(HoldReason),
}

/// Exact single edit relative to the previous authorized note snapshot.
pub struct EditorDelta<'a> {
    pub replaced: Range<usize>,
    pub inserted: &'a str,
}

fn valid_id(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 128
        && !value.chars().any(char::is_control)
}

fn valid_scope(value: &DocumentIdentity) -> bool {
    valid_id(&value.vault_id) && valid_id(&value.note_id)
}

fn digest(domain: &[u8], value: &str) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(domain);
    hash.update(&(value.len() as u64).to_le_bytes());
    hash.update(value.as_bytes());
    *hash.finalize().as_bytes()
}

fn doc_digest(value: &str) -> [u8; 32] {
    digest(b"akula-annotation-document-v1", value)
}

fn selection_digest(value: &str) -> [u8; 32] {
    digest(b"akula-annotation-span-v1", value)
}

fn valid_span(text: &str, start: usize, end: usize) -> bool {
    start < end && end <= text.len()
        && end - start <= MAX_ANNOTATED_SPAN_BYTES
        && text.is_char_boundary(start) && text.is_char_boundary(end)
}

/// Capture a word, partial word or phrase from a host-authorized editor.
/// Scope and annotation identity come from the host. No file IO or storage.
pub fn capture(
    document: &DocumentIdentity,
    annotation_id: &str,
    text: &str,
    selection: Range<usize>,
) -> Result<TextAnchor, AnchorError> {
    if !valid_scope(document) || !valid_id(annotation_id) {
        return Err(AnchorError::InvalidIdentity);
    }
    if text.len() > MAX_DOCUMENT_BYTES {
        return Err(AnchorError::TooMuchText);
    }
    if !valid_span(text, selection.start, selection.end) {
        return Err(AnchorError::InvalidRange);
    }
    let selected = &text[selection.clone()];
    if selected.trim().is_empty() {
        return Err(AnchorError::InvalidRange);
    }
    Ok(TextAnchor {
        annotation_id: annotation_id.to_owned(),
        document: document.clone(),
        byte_start: selection.start,
        byte_end: selection.end,
        selected_digest: selection_digest(selected),
        document_digest: doc_digest(text),
    })
}

fn shifted(position: usize, removed: usize, inserted: usize) -> Option<usize> {
    if inserted >= removed {
        position.checked_add(inserted - removed)
    } else {
        position.checked_sub(removed - inserted)
    }
}

/// Rebind only with the same document scope and a verified exact edit delta.
/// A different note with the same text cannot inherit an annotation.
/// The host must enforce File Gate/DocumentHub admission and revision CAS.
pub fn reattach(
    saved: &TextAnchor,
    document: &DocumentIdentity,
    previous: &str,
    current: &str,
    delta: Option<EditorDelta<'_>>,
) -> Result<Reattachment, AnchorError> {
    if !valid_scope(document) || !valid_scope(&saved.document)
        || !valid_id(&saved.annotation_id)
    {
        return Err(AnchorError::InvalidIdentity);
    }
    if document != &saved.document {
        return Err(AnchorError::ScopeMismatch);
    }
    if previous.len() > MAX_DOCUMENT_BYTES || current.len() > MAX_DOCUMENT_BYTES {
        return Err(AnchorError::TooMuchText);
    }
    if !valid_span(previous, saved.byte_start, saved.byte_end) {
        return Err(AnchorError::CorruptAnchor);
    }
    if doc_digest(previous) != saved.document_digest
        || selection_digest(&previous[saved.byte_start..saved.byte_end])
            != saved.selected_digest
    {
        return Ok(Reattachment::Hold(HoldReason::OldSnapshotChanged));
    }
    if previous == current {
        return Ok(Reattachment::Attached(saved.clone()));
    }
    let Some(delta) = delta else {
        return Ok(Reattachment::Hold(HoldReason::NoVerifiedEdit));
    };
    let start = delta.replaced.start;
    let end = delta.replaced.end;
    if start > end || end > previous.len()
        || !previous.is_char_boundary(start) || !previous.is_char_boundary(end)
    {
        return Ok(Reattachment::Hold(HoldReason::InvalidEdit));
    }
    let removed = end - start;
    let Some(new_size) = previous.len().checked_sub(removed)
        .and_then(|n| n.checked_add(delta.inserted.len())) else {
        return Ok(Reattachment::Hold(HoldReason::InvalidEdit));
    };
    let Some(next_suffix) = start.checked_add(delta.inserted.len()) else {
        return Ok(Reattachment::Hold(HoldReason::InvalidEdit));
    };
    if new_size != current.len()
        || current.get(..start) != previous.get(..start)
        || current.get(start..next_suffix) != Some(delta.inserted)
        || current.get(next_suffix..) != previous.get(end..)
    {
        return Ok(Reattachment::Hold(HoldReason::InvalidEdit));
    }

    let (next_start, next_end) = if end <= saved.byte_start {
        let Some(a) = shifted(saved.byte_start, removed, delta.inserted.len()) else {
            return Ok(Reattachment::Hold(HoldReason::InvalidEdit));
        };
        let Some(b) = shifted(saved.byte_end, removed, delta.inserted.len()) else {
            return Ok(Reattachment::Hold(HoldReason::InvalidEdit));
        };
        (a, b)
    } else if start >= saved.byte_end {
        (saved.byte_start, saved.byte_end)
    } else {
        return Ok(Reattachment::Hold(HoldReason::SelectionEdited));
    };

    if !valid_span(current, next_start, next_end)
        || selection_digest(&current[next_start..next_end]) != saved.selected_digest
    {
        return Ok(Reattachment::Hold(HoldReason::SelectionMismatch));
    }
    Ok(Reattachment::Attached(TextAnchor {
        annotation_id: saved.annotation_id.clone(),
        document: saved.document.clone(),
        byte_start: next_start,
        byte_end: next_end,
        selected_digest: saved.selected_digest,
        document_digest: doc_digest(current),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note() -> DocumentIdentity {
        DocumentIdentity { vault_id: "vault-A".into(), note_id: "note-A".into() }
    }

    fn selected(text: &str, phrase: &str) -> TextAnchor {
        let at = text.find(phrase).unwrap();
        capture(&note(), "highlight-1", text, at..at + phrase.len()).unwrap()
    }

    fn attached(result: Reattachment) -> TextAnchor {
        match result {
            Reattachment::Attached(anchor) => anchor,
            Reattachment::Hold(reason) => panic!("unexpected HOLD: {reason:?}"),
        }
    }

    #[test]
    fn supports_unicode_morpheme_and_long_phrase_without_storing_text() {
        let text = "Префикс — прекрасный; a piece of pizza";
        let partial = selected(text, "пре");
        assert_eq!(&text[partial.byte_start..partial.byte_end], "пре");
        assert_eq!(attached(reattach(&partial, &note(), text, text, None).unwrap()), partial);
        let phrase = selected(text, "a piece of");
        assert_eq!(&text[phrase.byte_start..phrase.byte_end], "a piece of");
    }

    #[test]
    fn unrelated_insertion_or_removal_before_span_shifts_utf8_offsets() {
        let old = "The courier ate pizza";
        let anchor = selected(old, "ate");
        let prefix = "Вчера: ";
        let new = format!("{prefix}{old}");
        let moved = attached(reattach(&anchor, &note(), old, &new,
            Some(EditorDelta { replaced: 0..0, inserted: prefix })).unwrap());
        assert_eq!(&new[moved.byte_start..moved.byte_end], "ate");
        assert_eq!(moved.byte_start, anchor.byte_start + prefix.len());
        let returned = attached(reattach(&moved, &note(), &new, old,
            Some(EditorDelta { replaced: 0..prefix.len(), inserted: "" })).unwrap());
        assert_eq!(returned.byte_start, anchor.byte_start);
        assert_eq!(returned.document_digest, anchor.document_digest);
    }

    #[test]
    fn insertion_after_span_keeps_offsets_and_updates_revision_fingerprint() {
        let old = "ate";
        let new = "ate — Past Simple";
        let original = selected(old, "ate");
        let next = attached(reattach(&original, &note(), old, new,
            Some(EditorDelta { replaced: old.len()..old.len(), inserted: " — Past Simple" })).unwrap());
        assert_eq!((next.byte_start, next.byte_end), (0, 3));
        assert_ne!(next.document_digest, original.document_digest);
    }

    #[test]
    fn editing_span_is_held_and_never_attaches_to_another_duplicate() {
        let old = "run run";
        let anchor = selected(old, "run");
        let new = "ran run";
        assert_eq!(reattach(&anchor, &note(), old, new,
            Some(EditorDelta { replaced: 0..3, inserted: "ran" })).unwrap(),
            Reattachment::Hold(HoldReason::SelectionEdited));
        assert_eq!(reattach(&anchor, &note(), old, "prefix run run", None).unwrap(),
            Reattachment::Hold(HoldReason::NoVerifiedEdit));
        let moved = attached(reattach(&anchor, &note(), old, "x run run",
            Some(EditorDelta { replaced: 0..0, inserted: "x " })).unwrap());
        assert_eq!(moved.byte_start, 2);
    }

    #[test]
    fn scope_and_revision_mismatch_fail_closed() {
        let old = "eat ate eaten";
        let a = selected(old, "ate");
        let other = DocumentIdentity { vault_id: "vault-B".into(), ..note() };
        assert_eq!(reattach(&a, &other, old, old, None),
            Err(AnchorError::ScopeMismatch));
        assert_eq!(reattach(&a, &note(), "eat was eaten", old, None).unwrap(),
            Reattachment::Hold(HoldReason::OldSnapshotChanged));
        let mut corrupted = a.clone();
        corrupted.byte_end = old.len() + 1;
        assert_eq!(reattach(&corrupted, &note(), old, old, None),
            Err(AnchorError::CorruptAnchor));
    }

    #[test]
    fn lies_about_editor_edit_are_refused_without_guessing() {
        let old = "The courier ate pizza";
        let anchor = selected(old, "ate");
        let new = "Now the courier ate pizza";
        assert_eq!(reattach(&anchor, &note(), old, new,
            Some(EditorDelta { replaced: 0..0, inserted: "Yesterday " })).unwrap(),
            Reattachment::Hold(HoldReason::InvalidEdit));
        assert_eq!(reattach(&anchor, &note(), old, new,
            Some(EditorDelta { replaced: 1..2, inserted: "Now " })).unwrap(),
            Reattachment::Hold(HoldReason::InvalidEdit));
    }

    #[test]
    fn rejects_invalid_byte_ranges_and_unbounded_text() {
        let text = "слово";
        assert_eq!(capture(&note(), "id", text, 1..4),
            Err(AnchorError::InvalidRange));
        assert_eq!(capture(&note(), "id", text, 0..0),
            Err(AnchorError::InvalidRange));
        assert_eq!(capture(&note(), "id", "  ", 0..2),
            Err(AnchorError::InvalidRange));
        assert_eq!(capture(&note(), "id", &"a".repeat(MAX_DOCUMENT_BYTES + 1), 0..1),
            Err(AnchorError::TooMuchText));
        assert_eq!(capture(&note(), "id", &"a".repeat(MAX_ANNOTATED_SPAN_BYTES + 1),
            0..MAX_ANNOTATED_SPAN_BYTES + 1),
            Err(AnchorError::InvalidRange));
        let invalid = DocumentIdentity { vault_id: "".into(), ..note() };
        assert_eq!(capture(&invalid, "id", text, 0..2),
            Err(AnchorError::InvalidIdentity));
    }
}
