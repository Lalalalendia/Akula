use super::TextEditor;
use crate::ui::keymap::{self, Command, Scope};
use masonry::core::keyboard::KeyboardEvent;

impl TextEditor {
    // Source-only chords are safe before the next layout/paint frame.
    // Geometry-dependent commands still require a measured document.
    pub(super) fn source_shortcut(&self, key: &KeyboardEvent) -> bool {
        self.table.is_none()
            && matches!(
                keymap::command(&[Scope::Text], &key.key, key.modifiers),
                Some(Command::SelectAll | Command::Copy | Command::Undo | Command::Redo)
            )
    }
}
