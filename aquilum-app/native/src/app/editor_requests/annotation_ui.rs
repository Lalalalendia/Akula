//! LANG-03d: explicit first-party annotation actions for the active note.
//!
//! A context-menu action is never a file grant. Both palette opening and
//! persistence must validate active workspace document identity, clean editor
//! state, canonical registered path and current backing-file checksum.
//! Saving metadata does not modify source Markdown or grant access to notes.

use std::ops::Range;
use std::path::PathBuf;

use aquilum_core::text::annotations::{
    capture, reattach, DocumentIdentity, Reattachment, MAX_DOCUMENT_BYTES,
};
use aquilum_core::text::annotation_store::{
    AnnotationRecord, AnnotationStore, BindingStatus, PaletteColor, StoreError,
};
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};
use masonry::peniko::Color;

use super::super::{App, Popup};
use crate::i18n::t;
use crate::ui::editor::{AnnotationPaintSpan, TextEditor};
use crate::ui::menu::MenuChoice;

const PALETTE: [PaletteColor; 6] = [
    PaletteColor::Yellow,
    PaletteColor::Green,
    PaletteColor::Violet,
    PaletteColor::Blue,
    PaletteColor::Orange,
    PaletteColor::Pink,
];

fn label(color: PaletteColor) -> &'static str {
    match color {
        PaletteColor::Yellow => "editor.annotation.yellow",
        PaletteColor::Green => "editor.annotation.green",
        PaletteColor::Violet => "editor.annotation.violet",
        PaletteColor::Blue => "editor.annotation.blue",
        PaletteColor::Orange => "editor.annotation.orange",
        PaletteColor::Pink => "editor.annotation.pink",
    }
}

fn paint(color: PaletteColor) -> Color {
    let (r, g, b) = match color {
        PaletteColor::Yellow => (255, 212, 72),
        PaletteColor::Green => (83, 184, 126),
        PaletteColor::Violet => (176, 124, 224),
        PaletteColor::Blue => (95, 159, 243),
        PaletteColor::Orange => (247, 155, 82),
        PaletteColor::Pink => (242, 133, 172),
    };
    Color::from_rgba8(r, g, b, 255)
}

pub(in crate::app) struct PendingAnnotation {
    path: PathBuf,
    document: DocumentIdentity,
    baseline_hash: String,
    body: String,
    selection: Range<usize>,
}

struct AuthorizedEditor {
    path: PathBuf,
    document: DocumentIdentity,
    baseline_hash: String,
    body: String,
    selection: Range<usize>,
}

impl App {
    /// Returns only the active registered Markdown document. A saved
    /// annotation ID/sidecar locator never grants permission to open a note.
    fn authorized_annotation_editor(&self, root: &RenderRoot) -> Option<AuthorizedEditor> {
        let session = self.session.as_ref()?;
        let note = self.note.as_ref()?;
        if self.tabs.active().path.as_deref() != Some(note.path.as_path()) {
            return None;
        }
        let baseline_hash = note.clean_cache_hash()?;
        let document_id = session.document(&self.workspace.core, &note.path)?;
        let canonical = session.study_document_path(&self.workspace.core, document_id)?;
        if std::fs::canonicalize(&note.path).ok()? != canonical
            || std::fs::metadata(&canonical).ok()?.len() > MAX_DOCUMENT_BYTES as u64
            || aquilum_core::files::document::read_file_hash_impl(&canonical).ok()?
                != baseline_hash
        {
            return None;
        }
        let editor = self.note_ids.as_ref()?.body.get(root)?;
        let body = editor.inner().text();
        if body.len() > MAX_DOCUMENT_BYTES || body != self.body().to_string() {
            return None;
        }
        Some(AuthorizedEditor {
            path: note.path.clone(),
            document: DocumentIdentity {
                vault_id: session.workspace_id.to_string(),
                note_id: document_id.to_string(),
            },
            baseline_hash,
            selection: editor.inner().selection(),
            body,
        })
    }

    pub(in crate::app) fn open_annotation_colors(&mut self, root: &mut RenderRoot) {
        let Some(source) = self.authorized_annotation_editor(root) else {
            self.study_notice(root, &t("editor.annotation.unavailable"));
            return;
        };
        if capture(&source.document, "pending", &source.body, source.selection.clone()).is_err() {
            self.study_notice(root, &t("editor.annotation.selectText"));
            return;
        }
        let choices = PALETTE.iter().enumerate()
            .map(|(i, color)| (i, t(label(*color)))).collect::<Vec<_>>();
        let layer = self.study_menu(root, &t("editor.annotation.chooseColor"), &choices);
        self.popup = Some(Popup::AnnotationColors {
            layer,
            pending: PendingAnnotation {
                path: source.path,
                document: source.document,
                baseline_hash: source.baseline_hash,
                body: source.body,
                selection: source.selection,
            },
        });
    }

    pub(in crate::app) fn on_annotation_colors(
        &mut self, root: &mut RenderRoot, layer: WidgetId,
        pending: PendingAnnotation, action: &ErasedAction,
    ) {
        let Some(choice) = action.downcast_ref::<MenuChoice>() else {
            self.popup = Some(Popup::AnnotationColors { layer, pending });
            return;
        };
        let MenuChoice::Item(index) = choice else {
            if matches!(choice, MenuChoice::Hover(_)) {
                self.popup = Some(Popup::AnnotationColors { layer, pending });
            }
            return;
        };
        root.remove_layer(layer);
        let Some(color) = PALETTE.get(*index) else { return };
        let valid = self.authorized_annotation_editor(root).is_some_and(|now|
            now.path == pending.path
                && now.document == pending.document
                && now.baseline_hash == pending.baseline_hash
                && now.body == pending.body
                && now.selection == pending.selection
        );
        if !valid {
            self.study_notice(root, &t("editor.annotation.changed"));
            return;
        }
        let annotation_id = uuid::Uuid::new_v4().to_string();
        let Ok(anchor) = capture(
            &pending.document, &annotation_id, &pending.body, pending.selection,
        ) else {
            self.study_notice(root, &t("editor.annotation.changed"));
            return;
        };
        let saved = (|| {
            let mut store = AnnotationStore::open(
                &self.workspace.data_dir.join("text-annotations.sqlite"),
            )?;
            let mut snapshot = store.load()?;
            snapshot.records.push(AnnotationRecord {
                anchor,
                color: *color,
                category: None,
                explanation: None,
                reference: None,
                status: BindingStatus::Current,
            });
            store.replace(snapshot.revision, &snapshot.records)?;
            Ok::<(), StoreError>(())
        })();
        if saved.is_err() {
            self.study_notice(root, &t("editor.annotation.saveFailed"));
            return;
        }
        self.refresh_annotation_overlay(root);
    }

    pub(in crate::app) fn hide_annotation_overlay(&mut self, root: &mut RenderRoot) {
        if let Some(ids) = &self.note_ids {
            ids.body.edit(root, |mut editor| {
                TextEditor::set_annotation_overlay(&mut editor, Vec::new());
            });
        }
    }

    /// Explicit refresh: a stored ID is not file access. Recheck the current
    /// active note and each digest; never attempt a fuzzy reattachment.
    pub(in crate::app) fn refresh_annotation_overlay(&mut self, root: &mut RenderRoot) {
        let Some(current) = self.authorized_annotation_editor(root) else {
            self.hide_annotation_overlay(root);
            self.study_notice(root, &t("editor.annotation.unavailable"));
            return;
        };
        let records = AnnotationStore::open(
            &self.workspace.data_dir.join("text-annotations.sqlite"),
        ).and_then(|store| store.load());
        let Ok(snapshot) = records else {
            self.hide_annotation_overlay(root);
            self.study_notice(root, &t("editor.annotation.saveFailed"));
            return;
        };
        let spans = snapshot.records.iter()
            .filter(|record| record.anchor.document == current.document
                && record.status == BindingStatus::Current)
            .filter_map(|record| match reattach(
                &record.anchor, &current.document, &current.body, &current.body, None,
            ) {
                Ok(Reattachment::Attached(anchor)) => Some(AnnotationPaintSpan {
                    bytes: anchor.byte_start..anchor.byte_end,
                    color: paint(record.color),
                }),
                _ => None,
            })
            .collect::<Vec<_>>();
        if let Some(ids) = &self.note_ids {
            ids.body.edit(root, |mut editor| {
                TextEditor::set_annotation_overlay(&mut editor, spans);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_palette_colors_have_distinct_opaque_labels() {
        let mut labels = std::collections::BTreeSet::new();
        for c in PALETTE {
            assert!(labels.insert(label(c)));
            let _ = paint(c);
        }
        assert_eq!(labels.len(), 6);
    }
}
