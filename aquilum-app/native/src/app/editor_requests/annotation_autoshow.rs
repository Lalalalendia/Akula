//! LANG-03h: quietly restore verified highlight paint on note-page install.
//!
//! Do not open a sidecar on cold vaults, display error dialogs during normal
//! navigation, infer a location by matching words, or read a closed note.
//! Only the already-authorized current, clean editor may receive ranges.
//! On missing/foreign/corrupt/stale data the editor keeps its empty overlay.

use aquilum_core::text::annotations::{reattach, Reattachment};
use aquilum_core::text::annotation_store::{AnnotationStore, BindingStatus};
use masonry::app::RenderRoot;

use super::super::App;
use super::annotation_ui::paint;
use crate::ui::editor::{AnnotationPaintSpan, TextEditor};

impl App {
    /// Called only after the native note page has been installed into
    /// RenderRoot. No file or note permission is created by this operation.
    pub(in crate::app) fn auto_show_annotations_after_note_open(
        &mut self, root: &mut RenderRoot,
    ) {
        // Do not create any database on first open. Also reject a symlink in
        // this read path; it must be a regular app-owned sidecar file.
        let path = self.workspace.data_dir.join("text-annotations.sqlite");
        if !std::fs::symlink_metadata(&path).ok()
            .is_some_and(|metadata| metadata.file_type().is_file())
        {
            return;
        }
        let Some(source) = self.authorized_annotation_editor(root) else {
            return;
        };
        let Some(snapshot) = AnnotationStore::open(&path)
            .and_then(|store| store.load()).ok()
        else {
            return;
        };
        let spans = snapshot.records.iter().filter(|record|
            record.anchor.document == source.document
                && record.status == BindingStatus::Current
        ).filter_map(|record| match reattach(
            &record.anchor, &source.document, &source.body, &source.body, None,
        ) {
            Ok(Reattachment::Attached(anchor)) => Some(AnnotationPaintSpan {
                bytes: anchor.byte_start..anchor.byte_end,
                color: paint(record.color),
            }),
            _ => None,
        }).collect::<Vec<_>>();
        if let Some(ids) = &self.note_ids {
            ids.body.edit(root, |mut editor| {
                TextEditor::set_annotation_overlay(&mut editor, spans);
            });
        }
    }
}
