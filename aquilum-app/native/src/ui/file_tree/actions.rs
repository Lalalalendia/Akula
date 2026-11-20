mod context;
mod dispatch;
use std::path::PathBuf;

use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};
use masonry::kurbo::Point;

use super::model::Modifiers;
use super::pinned::Unpin;
use super::row::RowAction;
use super::{FileTree, Press, TreeAction};
use crate::ui::editor::{TextAction, TextEditor};
use crate::ui::scroll::Scrolled;
use crate::ui::widgets::Pressed;

impl FileTree {
    fn on_row_action(&mut self, root: &mut RenderRoot, id: WidgetId, action: &RowAction) -> Option<TreeAction> {
        if self.model.renaming().is_some() {
            let name = self.rename.and_then(|field| field.get(root)).map(|f| f.inner().text());
            if let Some(name) = name {
                return self.commit_rename(root, &name);
            }
        }
        let index = self.first + self.row_ids.iter().position(|&row| row == id)?;
        let modifiers = match action {
            RowAction::Press(modifiers) => *modifiers,
            RowAction::DragStart(at) => {
                self.start_drag(root, index, *at);
                return None;
            }
            RowAction::DragMove(at) => {
                self.drag_to(root, *at);
                return None;
            }
            RowAction::Drop(at) => {
                self.drag_to(root, *at);
                return self.drop(root);
            }
            RowAction::ContextMenu(at) => return self.context_menu(root, index, *at),
        };
        match self.model.press(index, modifiers)? {
            Press::Rebuild => {
                self.save_expanded();
                self.refresh(root);
                None
            }
            Press::Open { path, new_tab } => Some(TreeAction::Open { path, new_tab }),
        }
    }

    pub fn start_rename(&mut self, root: &mut RenderRoot, path: PathBuf) {
        self.model.start_rename(path);
        self.rename = None;
        self.refresh(root);
        let Some(field) = self.rename else { return };
        field.edit(root, |mut field| {
            let len = field.widget.len();
            TextEditor::select_byte_range(&mut field, 0, len);
        });
        root.focus_on(Some(field.id()));
    }

    fn commit_rename(&mut self, root: &mut RenderRoot, input: &str) -> Option<TreeAction> {
        let from = self.model.renaming()?.to_path_buf();
        self.model.stop_rename();
        let Some((to, _)) = crate::file_ops::rename_target(&from, input) else {
            self.refresh(root);
            return None;
        };
        // Do not invent the destination row before the actual file move.
        self.refresh(root);
        Some(TreeAction::Rename { from, to })
    }
}
