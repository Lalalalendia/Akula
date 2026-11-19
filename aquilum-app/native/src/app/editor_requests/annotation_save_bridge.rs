//! LANG-03k: verified native edit history across the asynchronous note save.
//!
//! No annotation sidecar write is permitted while Markdown is dirty. This
//! module records exact active-editor deltas, waits for an acknowledged clean
//! native save, re-admits the same registered note and then applies SQLite CAS.
//! A missed edit, changed tab, conflicted save or stale sidecar discards the
//! entire volatile journal. No fuzzy matching or source Markdown mutation.

use std::ops::Range;
use std::path::PathBuf;

use aquilum_core::files::document::read_file_hash_impl;
use aquilum_core::text::annotation_chain::{VerifiedEditStep, MAX_VERIFIED_EDIT_STEPS};
use aquilum_core::text::annotation_reconcile::reconcile_sidecar;
use aquilum_core::text::annotation_store::{AnnotationStore, BindingStatus};
use aquilum_core::text::annotations::{DocumentIdentity, MAX_DOCUMENT_BYTES};
use masonry::app::RenderRoot;
use masonry::core::WidgetId;

use super::super::App;
use crate::ui::editor::Edit;

const MAX_VOLATILE_HISTORY_BYTES: usize = 6 * 1024 * 1024;

struct OwnedEdit {
    before: String,
    after: String,
    replaced: Range<usize>,
    inserted: String,
}

impl OwnedEdit {
    fn verified(&self) -> VerifiedEditStep<'_> {
        VerifiedEditStep {
            before: &self.before, after: &self.after,
            replaced: self.replaced.clone(), inserted: &self.inserted,
        }
    }
}

pub(in crate::app) struct PendingAnnotationRebind {
    path: PathBuf,
    document: DocumentIdentity,
    widget: WidgetId,
    sidecar_revision: u64,
    original: String,
    current: String,
    steps: Vec<OwnedEdit>,
    bytes: usize,
}

/// Replay every edit in exact widget emission order. Failure invalidates the
/// whole batch; the caller then drops the entire pending journal.
fn append_exact_edits(
    steps: &mut Vec<OwnedEdit>,
    bytes: &mut usize,
    before: &str,
    after: &str,
    edits: &[Edit],
) -> bool {
    if edits.is_empty() || before.len() > MAX_DOCUMENT_BYTES
        || after.len() > MAX_DOCUMENT_BYTES
        || steps.len().saturating_add(edits.len()) > MAX_VERIFIED_EDIT_STEPS
    {
        return false;
    }
    let mut current = before.to_owned();
    for edit in edits {
        if edit.range.start > edit.range.end
            || edit.range.end > current.len()
            || !current.is_char_boundary(edit.range.start)
            || !current.is_char_boundary(edit.range.end)
        {
            return false;
        }
        let mut next = current.clone();
        next.replace_range(edit.range.clone(), &edit.insert);
        let Some(cost) = current.len().checked_add(next.len())
            .and_then(|n| n.checked_add(edit.insert.len())) else {
            return false;
        };
        let Some(total) = bytes.checked_add(cost) else {
            return false;
        };
        if total > MAX_VOLATILE_HISTORY_BYTES || next.len() > MAX_DOCUMENT_BYTES {
            return false;
        }
        *bytes = total;
        steps.push(OwnedEdit {
            before: current, after: next.clone(),
            replaced: edit.range.clone(), inserted: edit.insert.clone(),
        });
        current = next;
    }
    current == after
}

impl App {
    /// Called before the new editor Rope replaces the previous App.text.
    pub(in crate::app) fn capture_annotation_edits(
        &mut self, root: &RenderRoot, edits: &[Edit],
    ) {
        let Some(note) = self.note.as_ref() else {
            self.pending_annotation_rebind = None;
            return;
        };
        let Some(ids) = self.note_ids.as_ref() else {
            self.pending_annotation_rebind = None;
            return;
        };
        let widget = ids.body.id();
        let Some(next) = ids.body.get(root).map(|editor| editor.inner().text()) else {
            self.pending_annotation_rebind = None;
            return;
        };
        let previous = self.body().to_string();

        if let Some(mut pending) = self.pending_annotation_rebind.take() {
            if pending.path != note.path || pending.widget != widget
                || pending.current != previous || note.save_error.is_some()
                || !append_exact_edits(
                    &mut pending.steps, &mut pending.bytes, &previous, &next, edits,
                )
            {
                return;
            }
            pending.current = next;
            self.pending_annotation_rebind = Some(pending);
            return;
        }

        // Only a current hash-verified active registered note can start a
        // journal; note.clean_cache_hash is None during a pending save.
        let Some(clean_hash) = note.clean_cache_hash() else { return; };
        if self.tabs.active().path.as_deref() != Some(note.path.as_path()) {
            return;
        }
        let Some(session) = self.session.as_ref() else { return; };
        let Some(document_id) = session.document(&self.workspace.core, &note.path) else {
            return;
        };
        let Some(authorized) = session.study_document_path(&self.workspace.core, document_id) else {
            return;
        };
        if std::fs::canonicalize(&note.path).ok().as_deref() != Some(authorized.as_path())
            || read_file_hash_impl(&authorized).ok().as_deref() != Some(clean_hash.as_str())
        {
            return;
        }
        let document = DocumentIdentity {
            vault_id: session.workspace_id.to_string(),
            note_id: document_id.to_string(),
        };
        let db = self.workspace.data_dir.join("text-annotations.sqlite");
        if !std::fs::symlink_metadata(&db).ok()
            .is_some_and(|meta| meta.file_type().is_file())
        {
            return;
        }
        let Ok(store) = AnnotationStore::open(&db) else { return; };
        let Ok(snapshot) = store.load() else { return; };
        if !snapshot.records.iter().any(|row|
            row.anchor.document == document && row.status == BindingStatus::Current
        ) {
            return;
        }
        let mut steps = Vec::new();
        let mut bytes = 0;
        if !append_exact_edits(&mut steps, &mut bytes, &previous, &next, edits) {
            return;
        }
        self.pending_annotation_rebind = Some(PendingAnnotationRebind {
            path: note.path.clone(), document, widget,
            sidecar_revision: snapshot.revision, original: previous,
            current: next, steps, bytes,
        });
    }

    /// Called after Note.tick has collected the native saver acknowledgement.
    pub(in crate::app) fn finish_annotation_rebind_after_save(
        &mut self, root: &mut RenderRoot,
    ) {
        let Some(pending) = self.pending_annotation_rebind.as_ref() else { return; };
        let Some(note) = self.note.as_ref() else {
            self.pending_annotation_rebind = None;
            return;
        };
        if note.path != pending.path
            || self.note_ids.as_ref().map(|ids| ids.body.id()) != Some(pending.widget)
            || note.save_error.is_some()
        {
            self.pending_annotation_rebind = None;
            return;
        }
        if note.clean_cache_hash().is_none() {
            return; // save still queued; no SQLite write
        }
        let authorized = self.authorized_annotation_editor(root);
        let pending = self.pending_annotation_rebind.take().expect("pending checked");
        let Some(current) = authorized else { return; };
        if current.path != pending.path
            || current.document != pending.document
            || current.body != pending.current
            || self.note_ids.as_ref().map(|ids| ids.body.id()) != Some(pending.widget)
        {
            return;
        }
        let verified = pending.steps.iter().map(OwnedEdit::verified).collect::<Vec<_>>();
        let db = self.workspace.data_dir.join("text-annotations.sqlite");
        if !std::fs::symlink_metadata(&db).ok()
            .is_some_and(|meta| meta.file_type().is_file())
        {
            return;
        }
        let succeeded = AnnotationStore::open(&db).ok().is_some_and(|mut store| {
            reconcile_sidecar(
                &mut store, pending.sidecar_revision, &pending.document,
                &pending.original, &pending.current, &verified,
            ).is_ok()
        });
        if succeeded {
            self.refresh_annotation_overlay(root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(range: Range<usize>, text: &str) -> Edit {
        Edit { range, insert: text.into() }
    }

    #[test]
    fn replay_unicode_and_consecutive_edits_without_guessing_offsets() {
        let mut chain = Vec::new();
        let mut bytes = 0;
        assert!(append_exact_edits(&mut chain, &mut bytes, "run run",
            "Сначала run! run", &[
                edit(0..0, "Сначала "),
                edit("Сначала run".len().."Сначала run".len(), "!"),
            ]));
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].after, "Сначала run run");
        assert_eq!(chain[1].after, "Сначала run! run");
    }

    #[test]
    fn malformed_or_unreported_delta_cannot_be_partially_accepted() {
        let mut chain = Vec::new();
        let mut bytes = 0;
        assert!(!append_exact_edits(&mut chain, &mut bytes, "run",
            "ran", &[edit(0..0, "x")]));
        assert!(!append_exact_edits(&mut chain, &mut bytes, "слово",
            "xслово", &[edit(1..1, "x")]));
    }

    #[test]
    fn history_limit_and_memory_limit_discard_unverified_journal() {
        let mut chain = Vec::new();
        let mut bytes = 0;
        let mut current = "run".to_owned();
        for _ in 0..MAX_VERIFIED_EDIT_STEPS {
            let next = format!("{current}!");
            assert!(append_exact_edits(&mut chain, &mut bytes, &current, &next,
                &[edit(current.len()..current.len(), "!")]));
            current = next;
        }
        assert!(!append_exact_edits(&mut chain, &mut bytes, &current,
            &format!("{current}!"), &[edit(current.len()..current.len(), "!")]));

        let big = "x".repeat(1024 * 1024);
        let mut huge = Vec::new();
        let mut size = 0;
        for _ in 0..4 {
            let _ = append_exact_edits(&mut huge, &mut size, &big,
                &format!("{big}y"), &[edit(big.len()..big.len(), "y")]);
        }
        assert!(size <= MAX_VOLATILE_HISTORY_BYTES);
        assert!(huge.len() < 4);
    }
}
