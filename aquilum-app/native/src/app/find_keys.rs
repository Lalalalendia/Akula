use masonry::app::RenderRoot;
use masonry::core::keyboard::{Key, Modifiers};
use crate::ui::keymap::{self, Command, Scope};
use super::App;

impl App {
    pub fn find_key(&mut self, root: &mut RenderRoot, key: &Key, modifiers: Modifiers) -> bool {
        let Some(find) = &self.find else { return false };
        if root.focused_widget() != Some(find.field.id()) {
            return false;
        }
        match keymap::command(&[Scope::Find], key, modifiers) {
            Some(Command::FindNext) => self.find_step(root, 1),
            Some(Command::FindPrevious) => self.find_step(root, -1),
            Some(Command::Close) => self.close_find(root),
            _ => return false,
        }
        true
    }
}
