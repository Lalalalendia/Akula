//! Read-only bridge from the active native table to LANG-01.
//! Always uses the live editor model, including unsaved changes.

use super::*;
use aquilum_core::learning::editor_snapshot::{self, CaptureError, EditorTableSnapshot};

impl TextEditor {
    /// Current table cells only: the range is ephemeral and does not grant
    /// access to another note or establish a stable source identity.
    pub fn study_table_snapshot(&self) -> Result<EditorTableSnapshot, CaptureError> {
        let session = self.table.as_ref().ok_or(CaptureError::NoActiveTable)?;
        let (range, view) = self.doc.table_at(session.start)
            .ok_or(CaptureError::NoActiveTable)?;
        editor_snapshot::capture(
            &view.model.cells,
            range,
            self.doc.len(),
            !view.model.merges.is_empty(),
        )
    }
}
