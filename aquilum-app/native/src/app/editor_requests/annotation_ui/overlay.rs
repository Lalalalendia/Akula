use aquilum_core::text::annotations::{reattach, Reattachment};
use aquilum_core::text::annotation_store::{AnnotationRecord, BindingStatus};
use masonry::app::RenderRoot;
use crate::app::App;
use crate::ui::editor::{AnnotationPaintSpan, TextEditor};
use super::{AuthorizedEditor, paint};

impl App {
    // Both callers independently authorize the editor and load their sidecar.
    pub(in crate::app::editor_requests) fn show_verified_annotation_overlay(
        &self, root: &mut RenderRoot, current: &AuthorizedEditor, records: &[AnnotationRecord],
    ) {
        let spans = records.iter()
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
