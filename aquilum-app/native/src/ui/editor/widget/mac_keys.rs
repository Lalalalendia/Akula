use super::*;
use crate::ui::keymap::Command;

impl TextEditor {
    pub(super) fn vertical_target(&mut self, command: Option<Command>, down: bool) -> (usize, Affinity) {
        match command {
            Some(Command::DocumentStart) => (0, Affinity::Downstream),
            Some(Command::DocumentEnd) => (self.doc.len(), Affinity::Downstream),
            _ => self.vertical(down),
        }
    }

    pub(super) fn backspace_with_command(&mut self, command: Option<Command>) {
        if command == Some(Command::LineDeleteBackward) {
            self.delete_to_line_start();
        } else {
            self.delete(false, command == Some(Command::DeleteWord));
        }
    }

    pub(super) fn delete_to_line_start(&mut self) {
        let range = self.selection();
        if !range.is_empty() {
            self.replace(range, "", Kind::Other);
            return;
        }
        let begin = self.line_edge(false).0;
        if begin < self.focus {
            self.replace(begin..self.focus, "", Kind::Deleting);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_chords_ignore_visual_column_and_return_document_boundaries() {
        let mut editor = TextEditor::new("first\nsecond");
        assert_eq!(editor.vertical_target(Some(Command::DocumentStart), true).0, 0);
        assert_eq!(editor.vertical_target(Some(Command::DocumentEnd), false).0, editor.doc.len());
    }

    #[test]
    fn command_backspace_removes_to_visual_line_start_without_erasing_other_lines() {
        let mut editor = TextEditor::new("first\nsecond");
        editor.focus = editor.doc.len();
        editor.anchor = editor.focus;
        editor.backspace_with_command(Some(Command::LineDeleteBackward));
        assert_eq!(editor.text(), "first\n");
    }

    #[test]
    fn command_backspace_respects_existing_selection() {
        let mut editor = TextEditor::new("alpha beta");
        editor.anchor = 0;
        editor.focus = 5;
        editor.backspace_with_command(Some(Command::LineDeleteBackward));
        assert_eq!(editor.text(), " beta");
    }
}
