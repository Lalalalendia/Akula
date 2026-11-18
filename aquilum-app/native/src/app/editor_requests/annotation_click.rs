//! LANG-03g: resolve a direct painted-span click against the active note.
//!
//! Editor paint can request a preview, but cannot grant access by itself.
//! Recheck the active registered editor, its canonical backing-file hash,
//! document identity and exact source span digest on every click.

use std::ops::Range;

use aquilum_core::text::annotation_store::{AnnotationStore, AnnotationRecord};
use masonry::app::RenderRoot;
use masonry::core::WidgetId;

use super::super::App;
use super::annotation_ui::AuthorizedEditor;
use super::annotation_preview::trusted_explanation;
use crate::i18n::t;

fn selected_explanation<'a>(
    records: &'a [AnnotationRecord],
    editor: &AuthorizedEditor,
    range: Range<usize>,
) -> Option<Option<&'a str>> {
    if range.start >= range.end
        || range.end > editor.body.len()
        || !editor.body.is_char_boundary(range.start)
        || !editor.body.is_char_boundary(range.end)
    {
        return None;
    }
    let selected = AuthorizedEditor {
        path: editor.path.clone(),
        document: editor.document.clone(),
        baseline_hash: editor.baseline_hash.clone(),
        body: editor.body.clone(),
        selection: range,
    };
    let result = trusted_explanation(records, &selected);
    result
}

impl App {
    pub(in crate::app) fn preview_annotation_on_click(
        &mut self, root: &mut RenderRoot,
        editor: WidgetId, range: Range<usize>,
    ) {
        // Another tab's pending click must never open a note in this tab.
        if self.note_ids.as_ref().map(|ids| ids.body.id()) != Some(editor) {
            return;
        }
        let Some(active) = self.authorized_annotation_editor(root) else {
            return;
        };
        let state = AnnotationStore::open(
            &self.workspace.data_dir.join("text-annotations.sqlite"),
        ).and_then(|store| store.load());
        let Ok(snapshot) = state else { return; };
        let Some(explanation) = selected_explanation(&snapshot.records, &active, range)
            else { return; };
        let message = explanation.filter(|text| !text.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| t("editor.annotation.emptyExplanation"));
        self.study_notice(root, &message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aquilum_core::text::annotations::{capture, DocumentIdentity};
    use aquilum_core::text::annotation_store::{AnnotationRecord, BindingStatus, PaletteColor};

    fn active(text: &str) -> AuthorizedEditor {
        AuthorizedEditor {
            path: "note.md".into(),
            document: DocumentIdentity {
                vault_id: "vault".into(), note_id: "note".into(),
            },
            baseline_hash: "h".into(),
            body: text.into(),
            selection: 0..0,
        }
    }

    fn annotation(text: &str, range: Range<usize>) -> AnnotationRecord {
        let editor = active(text);
        AnnotationRecord {
            anchor: capture(&editor.document, "ann-1", text, range).unwrap(),
            color: PaletteColor::Yellow,
            category: None,
            explanation: Some("Past tense of eat".into()),
            reference: None,
            status: BindingStatus::Current,
        }
    }

    #[test]
    fn click_opens_only_exact_current_annotation_not_identical_other_word() {
        let editor = active("eat eat");
        let records = [annotation("eat eat", 0..3)];
        assert_eq!(selected_explanation(&records, &editor, 0..3),
            Some(Some("Past tense of eat")));
        assert_eq!(selected_explanation(&records, &editor, 4..7), None);
        assert_eq!(selected_explanation(&records, &active("ate eat"), 0..3), None);
    }

    #[test]
    fn rejects_foreign_vault_stale_range_and_unbound_annotation() {
        let editor = active("eat eat");
        let mut a = annotation("eat eat", 0..3);
        a.anchor.document.vault_id = "foreign".into();
        assert_eq!(selected_explanation(&[a], &editor, 0..3), None);
        let mut held = annotation("eat eat", 0..3);
        held.status = BindingStatus::NeedsRebind;
        assert_eq!(selected_explanation(&[held], &editor, 0..3), None);
        let records = [annotation("eat eat", 0..3)];
        assert_eq!(selected_explanation(&records, &editor, 2..9), None);
        assert_eq!(selected_explanation(&records, &editor, 1..2), None);
    }
}
