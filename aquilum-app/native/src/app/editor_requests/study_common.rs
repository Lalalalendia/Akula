//! Shared display and dialog transitions. No source validation or persistence policy.
use aquilum_core::learning::state::StateError;
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};
use crate::i18n::{t, t_with};
use crate::ui::dialog::{DialogAnswer, DialogIds};
use crate::app::App;
use super::super::study_add::PendingColumns;

pub(in crate::app::editor_requests) fn bounded_text(value: &str, limit: usize) -> String {
    let mut visible = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if visible.chars().count() > limit {
        visible = visible.chars().take(limit).collect::<String>() + "…";
    }
    visible
}

// Feedback dialogs are removed immediately on an answer or a Closed event.
// Input dialogs and animated preview closing deliberately use their own paths.
pub(in crate::app::editor_requests) fn feedback_answer(root: &mut RenderRoot, id: WidgetId, dialog: &DialogIds, action: &ErasedAction) -> Option<DialogAnswer> {
    let answer = if action.downcast_ref::<DialogAnswer>() == Some(&DialogAnswer::Closed) {
        Some(DialogAnswer::Closed)
    } else {
        dialog.answer(id, action)
    };
    if answer.is_some() {
        root.remove_layer(dialog.layer);
    }
    answer
}

impl App {
    pub(in crate::app::editor_requests) fn bind_live_table_with_notice(&mut self, root: &mut RenderRoot, pending: &PendingColumns, id: &str, name: &str) {
        match self.bind_live_table(root, pending, id, name) {
            Ok(()) => self.study_add_notice(root, &t_with("editor.table.studySaved", &[("name", name)]), true),
            Err(StateError::DuplicateTable | StateError::NeedsRebind) =>
                self.study_add_notice(root, &t("editor.table.studyHold"), false),
            Err(StateError::InvalidSelection) =>
                self.study_add_notice(root, &t("editor.table.studyStillChanged"), false),
            Err(_) => self.study_add_notice(root, &t("editor.table.studySaveFailed"), false),
        }
    }
}
