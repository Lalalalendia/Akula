//! LANG-03f: read-only explanation preview for one verified saved highlight.
//!
//! No file grants, source-text copies or sidecar writes. Access to a record's
//! explanation requires fresh active-note admission and a matching live span.

use aquilum_core::text::annotation_store::{AnnotationRecord, AnnotationStore};
use masonry::app::RenderRoot;

use super::super::App;
use super::annotation_explain::matching_record;
use super::annotation_ui::AuthorizedEditor;
use crate::i18n::t;

pub(in crate::app) fn trusted_explanation<'a>(
    records: &'a [AnnotationRecord],
    active: &AuthorizedEditor,
) -> Option<Option<&'a str>> {
    matching_record(records, active).map(|record| record.explanation.as_deref())
}

impl App {
    /// Read-only context-menu preview, not a hover/click-on-span tooltip.
    /// A stored ID is never sufficient to read any note's metadata.
    pub(in crate::app) fn preview_annotation_explanation(
        &mut self, root: &mut RenderRoot,
    ) {
        let Some(active) = self.authorized_annotation_editor(root) else {
            self.study_notice(root, &t("editor.annotation.unavailable"));
            return;
        };
        let state = AnnotationStore::open(
            &self.workspace.data_dir.join("text-annotations.sqlite"),
        ).and_then(|store| store.load());
        let Ok(state) = state else {
            self.study_notice(root, &t("editor.annotation.saveFailed"));
            return;
        };
        let Some(explanation) = trusted_explanation(&state.records, &active) else {
            self.study_notice(root, &t("editor.annotation.selectSaved"));
            return;
        };
        let description = explanation.filter(|s| !s.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| t("editor.annotation.emptyExplanation"));
        self.study_notice(root, &description);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aquilum_core::text::annotations::{capture, DocumentIdentity};
    use aquilum_core::text::annotation_store::{AnnotationRecord, BindingStatus, PaletteColor};
    use std::path::PathBuf;

    fn active(text: &str, vault_id: &str) -> AuthorizedEditor {
        AuthorizedEditor {
            path: PathBuf::from("lesson.md"),
            document: DocumentIdentity {
                vault_id: vault_id.into(), note_id: "registered-note".into(),
            },
            baseline_hash: "validated-disk-hash".into(),
            body: text.into(),
            selection: 0..3,
        }
    }

    fn record(text: &str, explanation: Option<&str>) -> AnnotationRecord {
        let current = active(text, "vault-A");
        AnnotationRecord {
            anchor: capture(&current.document, "opaque-1", text, 0..3).unwrap(),
            color: PaletteColor::Yellow,
            category: None,
            explanation: explanation.map(str::to_owned),
            reference: None,
            status: BindingStatus::Current,
        }
    }

    #[test]
    fn exact_highlight_returns_only_user_written_explanation() {
        let editor = active("run run", "vault-A");
        let notes = [record("run run", Some("Past tense: ran"))];
        assert_eq!(trusted_explanation(&notes, &editor), Some(Some("Past tense: ran")));
        assert_eq!(trusted_explanation(&notes, &active("ran run", "vault-A")), None);
        assert_eq!(trusted_explanation(&notes, &active("run run", "vault-B")), None);
    }

    #[test]
    fn empty_note_is_distinct_from_missing_or_ambiguous_highlight() {
        let editor = active("run run", "vault-A");
        let saved = record("run run", None);
        assert_eq!(trusted_explanation(&[saved.clone()], &editor), Some(None));
        assert_eq!(trusted_explanation(&[saved.clone(), saved], &editor), None);
        assert_eq!(trusted_explanation(&[], &editor), None);
    }

    #[test]
    fn held_or_overlapping_record_does_not_leak_explanation() {
        let editor = active("run run", "vault-A");
        let mut held = record("run run", Some("don't show"));
        held.status = BindingStatus::NeedsRebind;
        assert_eq!(trusted_explanation(&[held], &editor), None);
        let mut moved = record("run run", Some("other word"));
        moved.anchor.byte_start = 4;
        moved.anchor.byte_end = 7;
        assert_eq!(trusted_explanation(&[moved], &editor), None);
    }
}
