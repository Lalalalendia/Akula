//! LANG-03e: manual explanation editing for an authorized saved color span.
//!
//! The source text stays in Markdown. User-authored explanation is metadata
//! in app-data SQLite, edited only with full revision CAS and a fresh clean
//! active-note check. The record ID alone never grants document read access.

use aquilum_core::text::annotations::{reattach, Reattachment, TextAnchor};
use aquilum_core::text::annotation_store::{
    AnnotationRecord, AnnotationStore, BindingStatus, Snapshot, StoreError,
};
use masonry::accesskit::Role;
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, NewWidget, PropertySet, WidgetId};
use masonry::kurbo::{Axis, Point};
use masonry::layout::Length;
use masonry::properties::types::{CrossAxisAlignment, MainAxisAlignment};
use masonry::properties::{Gap, Padding};
use masonry::widgets::Flex;

use super::super::{App, Popup};
use super::annotation_ui::AuthorizedEditor;
use crate::i18n::t;
use crate::ui::dialog::{self, CardSize, DialogAnswer, DialogIds, Modal};
use crate::ui::editor::{TextAction, TextEditor};
use crate::ui::handle::Handle;
use crate::ui::search::{self, Placeholder};
use crate::ui::theme;
use crate::ui::tokens::size;
use crate::ui::widgets::{Rule, TextButton, Variant};

pub(in crate::app) struct PendingExplanation {
    source: AuthorizedEditor,
    anchor: TextAnchor,
    revision: u64,
}

const MAX_EXPLANATION_BYTES: usize = 4096;

pub(in crate::app) fn matching_record<'a>(
    records: &'a [AnnotationRecord], current: &AuthorizedEditor,
) -> Option<&'a AnnotationRecord> {
    let mut valid = records.iter().filter(|record| {
        record.anchor.document == current.document
            && record.status == BindingStatus::Current
            && record.anchor.byte_start == current.selection.start
            && record.anchor.byte_end == current.selection.end
            && matches!(reattach(
                &record.anchor, &current.document,
                &current.body, &current.body, None,
            ), Ok(Reattachment::Attached(_)))
    });
    let one = valid.next()?;
    if valid.next().is_some() { return None; }
    Some(one)
}

fn clean_explanation(value: &str) -> Option<Option<String>> {
    if value.len() > MAX_EXPLANATION_BYTES || value.contains('\0') {
        return None;
    }
    let cleaned = value.trim();
    Some((!cleaned.is_empty()).then(|| cleaned.to_owned()))
}

fn save_note(
    store: &mut AnnotationStore, pending: &PendingExplanation,
    current: &AuthorizedEditor, value: &str,
) -> Result<(), StoreError> {
    let explanation = clean_explanation(value).ok_or(StoreError::InvalidRecord)?;
    if current.path != pending.source.path
        || current.document != pending.source.document
        || current.baseline_hash != pending.source.baseline_hash
        || current.body != pending.source.body
        || current.selection != pending.source.selection
    {
        return Err(StoreError::RevisionConflict);
    }
    let mut snapshot: Snapshot = store.load()?;
    if snapshot.revision != pending.revision {
        return Err(StoreError::RevisionConflict);
    }
    let record = snapshot.records.iter_mut()
        .find(|record| record.anchor.annotation_id == pending.anchor.annotation_id
            && record.anchor.document == pending.anchor.document)
        .ok_or(StoreError::RevisionConflict)?;
    if record.anchor != pending.anchor
        || record.status != BindingStatus::Current
        || !matches!(reattach(&record.anchor,
            &current.document, &current.body, &current.body, None),
            Ok(Reattachment::Attached(_)))
    {
        return Err(StoreError::RevisionConflict);
    }
    record.explanation = explanation;
    store.replace(pending.revision, &snapshot.records)?;
    Ok(())
}

impl App {
    pub(in crate::app) fn open_annotation_explanation(
        &mut self, root: &mut RenderRoot,
    ) {
        let Some(source) = self.authorized_annotation_editor(root) else {
            self.study_notice(root, &t("editor.annotation.unavailable"));
            return;
        };
        let path = self.workspace.data_dir.join("text-annotations.sqlite");
        let snapshot = AnnotationStore::open(&path).and_then(|store| store.load());
        let Ok(snapshot) = snapshot else {
            self.study_notice(root, &t("editor.annotation.saveFailed"));
            return;
        };
        let Some(record) = matching_record(&snapshot.records, &source) else {
            self.study_notice(root, &t("editor.annotation.selectSaved"));
            return;
        };
        let previous = record.explanation.clone().unwrap_or_default();
        let pending = PendingExplanation {
            source, anchor: record.anchor.clone(), revision: snapshot.revision,
        };
        let title = t("editor.annotation.explainTitle");
        let header = dialog::header(&title, None::<NewWidget<Flex>>);
        let (entry, field, placeholder) =
            search::field(&previous, t("editor.annotation.explainHint"));
        let content = NewWidget::new(Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch).with_fixed(entry))
            .with_props(PropertySet::new()
                .with(Padding::all(Length::px(size::SPACE_24))));
        let cancel = NewWidget::new(TextButton::new(
            t("editor.annotation.cancel"), Variant::Ghost,
        ));
        let confirm = NewWidget::new(TextButton::new(
            t("editor.annotation.saveNote"), Variant::Primary,
        ));
        let ids = (cancel.id(), confirm.id());
        let footer = NewWidget::new(Flex::row()
            .main_axis_alignment(MainAxisAlignment::End)
            .with_fixed(cancel).with_fixed(confirm))
            .with_props(PropertySet::new()
                .with(Gap::new(Length::px(size::DIALOG_GAP)))
                .with(Padding::all(Length::px(size::DIALOG_PADDING))));
        let border = || Rule::new(Axis::Horizontal, || theme::current().border_default);
        let card = NewWidget::new(Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_fixed(header).with_fixed(border())
            .with_fixed(content).with_fixed(border()).with_fixed(footer));
        let modal = Modal::new(card, &title,
            CardSize::Fit(size::CONFIRM_DIALOG_WIDTH), Role::Dialog, true);
        let dialog = DialogIds::of(&modal, ids);
        root.add_layer(modal, Point::ORIGIN);
        root.focus_on(Some(field.id()));
        self.popup = Some(Popup::AnnotationExplanation {
            dialog, field, placeholder, pending, value: previous,
        });
    }

    pub(in crate::app) fn on_annotation_explanation(
        &mut self, root: &mut RenderRoot, id: WidgetId, dialog: DialogIds,
        field: Handle<TextEditor>, placeholder: Handle<Placeholder>,
        pending: PendingExplanation, mut value: String, action: &ErasedAction,
    ) {
        if action.downcast_ref::<DialogAnswer>() == Some(&DialogAnswer::Closed) {
            root.remove_layer(dialog.layer);
            return;
        }
        if id == field.id() {
            match action.downcast_ref::<TextAction>() {
                Some(TextAction::Changed(text)) => {
                    value = text.clone();
                    placeholder.edit(root, |mut p|
                        Placeholder::set_empty(&mut p, value.is_empty()));
                }
                Some(TextAction::Entered(text)) => {
                    root.remove_layer(dialog.layer);
                    self.commit_annotation_explanation(root, pending, text);
                    return;
                }
                Some(TextAction::Cancelled) => {
                    root.remove_layer(dialog.layer);
                    return;
                }
                _ => {}
            }
            self.popup = Some(Popup::AnnotationExplanation {
                dialog, field, placeholder, pending, value,
            });
            return;
        }
        let Some(answer) = dialog.answer(id, action) else {
            self.popup = Some(Popup::AnnotationExplanation {
                dialog, field, placeholder, pending, value,
            });
            return;
        };
        root.remove_layer(dialog.layer);
        if answer == DialogAnswer::Confirm {
            self.commit_annotation_explanation(root, pending, &value);
        }
    }

    fn commit_annotation_explanation(
        &mut self, root: &mut RenderRoot,
        pending: PendingExplanation, value: &str,
    ) {
        let Some(current) = self.authorized_annotation_editor(root) else {
            self.study_notice(root, &t("editor.annotation.changed"));
            return;
        };
        let result = AnnotationStore::open(
            &self.workspace.data_dir.join("text-annotations.sqlite"),
        ).and_then(|mut store| save_note(&mut store, &pending, &current, value));
        match result {
            Ok(()) => self.study_notice(root, &t("editor.annotation.noteSaved")),
            Err(StoreError::RevisionConflict) =>
                self.study_notice(root, &t("editor.annotation.changed")),
            Err(_) => self.study_notice(root, &t("editor.annotation.saveFailed")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aquilum_core::text::annotations::{capture, DocumentIdentity};
    use aquilum_core::text::annotation_store::PaletteColor;

    fn authorized(body: &str) -> AuthorizedEditor {
        AuthorizedEditor {
            path: "lesson.md".into(),
            document: DocumentIdentity {
                vault_id: "vault-1".into(), note_id: "note-1".into(),
            },
            baseline_hash: "hash-1".into(),
            body: body.into(),
            selection: 0..3,
        }
    }

    #[test]
    fn edits_user_note_with_revision_cas_without_copying_source() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = AnnotationStore::open(&dir.path().join("notes.sqlite")).unwrap();
        let editor = authorized("run run");
        let anchor = capture(&editor.document, "a-1", &editor.body, 0..3).unwrap();
        let record = AnnotationRecord {
            anchor: anchor.clone(), color: PaletteColor::Yellow,
            category: None, explanation: None, reference: None,
            status: BindingStatus::Current,
        };
        store.replace(0, &[record]).unwrap();
        let pending = PendingExplanation {
            source: authorized("run run"), anchor, revision: 1,
        };
        assert_eq!(save_note(&mut store, &pending, &editor, "  бежать  "), Ok(()));
        let snap = store.load().unwrap();
        assert_eq!(snap.revision, 2);
        assert_eq!(snap.records[0].explanation.as_deref(), Some("бежать"));
        assert_ne!(snap.records[0].anchor.selected_digest, [0; 32]);
    }

    #[test]
    fn stale_writer_and_changed_source_cannot_mutate_explanation() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = AnnotationStore::open(&dir.path().join("notes.sqlite")).unwrap();
        let editor = authorized("run run");
        let anchor = capture(&editor.document, "a-1", &editor.body, 0..3).unwrap();
        let record = AnnotationRecord {
            anchor: anchor.clone(), color: PaletteColor::Blue,
            category: None, explanation: None, reference: None,
            status: BindingStatus::Current,
        };
        store.replace(0, &[record]).unwrap();
        let pending = PendingExplanation { source: authorized("run run"), anchor, revision: 1 };
        let changed = authorized("ran run");
        assert_eq!(save_note(&mut store, &pending, &changed, "wrong"),
            Err(StoreError::RevisionConflict));
        store.replace(1, &[]).unwrap();
        assert_eq!(save_note(&mut store, &pending, &editor, "wrong"),
            Err(StoreError::RevisionConflict));
        assert!(store.load().unwrap().records.is_empty());
    }

    #[test]
    fn rejects_missing_record_and_unbounded_explanation() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = AnnotationStore::open(&dir.path().join("notes.sqlite")).unwrap();
        let editor = authorized("run run");
        let anchor = capture(&editor.document, "a-1", &editor.body, 0..3).unwrap();
        let pending = PendingExplanation { source: authorized("run run"), anchor, revision: 0 };
        assert_eq!(save_note(&mut store, &pending, &editor, "a"),
            Err(StoreError::RevisionConflict));
        assert_eq!(clean_explanation(&"x".repeat(MAX_EXPLANATION_BYTES + 1)), None);
        assert_eq!(clean_explanation(" \0"), None);
        assert_eq!(clean_explanation("  "), Some(None));
    }
}
