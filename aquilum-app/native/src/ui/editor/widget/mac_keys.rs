use super::*;
use crate::ui::keymap::Command;

impl TextEditor {
    pub(super) fn vertical_target(&self, command: Option<Command>, down: bool) -> (usize, Affinity) {
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
