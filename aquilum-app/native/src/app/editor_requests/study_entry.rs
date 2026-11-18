//! Entry point for a safe study preview in the existing table context menu.
//! It intentionally only reads the current editor widget, not a file path.

use masonry::app::RenderRoot;
use super::super::App;
use crate::i18n::t;

impl App {
    pub(in crate::app) fn open_study_from_active_table(&mut self, root: &mut RenderRoot) {
        let snapshot = self.note_ids.as_ref()
            .and_then(|ids| ids.body.get(root))
            .map(|editor| editor.inner().study_table_snapshot());
        match snapshot {
            Some(Ok(snapshot)) => self.open_study_question(root, snapshot),
            _ => self.study_notice(root, &t("editor.table.studyUnavailable")),
        }
    }
}
