use super::*;
impl FileTree {
    pub fn on_action(&mut self, root: &mut RenderRoot, id: WidgetId, action: &ErasedAction) -> Option<TreeAction> {
        if self.on_filter_action(root, id, action) { return None; }
        if let Some(Scrolled(offset)) = action.downcast_ref::<Scrolled>() {
            if self.scroll.is_some_and(|s| s.id() == id) {
                self.offset = *offset;
                self.show_rows_from(root, *offset);
                self.update_pins(root);
            }
            return None;
        }
        if let Some(Unpin(index)) = action.downcast_ref::<Unpin>() {
            if self.model.press(*index, Modifiers::default()).is_some() {
                self.save_expanded();
                self.show_row(root, Some(*index));
            }
            return None;
        }
        if Some(id) == self.create_button && action.downcast_ref::<Pressed>().is_some() {
            return Some(TreeAction::CreateNote);
        }
        if let Some(toolbar) = &self.toolbar
            && action.downcast_ref::<Pressed>().is_some()
        {
            if id == toolbar.collapse {
                if self.model.collapse_all() {
                    self.save_expanded();
                    self.first = Self::first_near(self.offset);
                    self.refresh(root);
                }
                return None;
            }
            let folder = self.model.target_folder();
            if id == toolbar.new_note {
                return folder.map(|folder| TreeAction::NewNote { folder });
            }
            if id == toolbar.new_folder {
                return folder.map(|folder| TreeAction::NewFolder { folder });
            }
            if id == toolbar.sort {
                return Some(TreeAction::SortMenu { button: id });
            }
        }
        if let Some(text) = action.downcast_ref::<TextAction>() {
            return match text {
                TextAction::Entered(name) => self.commit_rename(root, name),
                TextAction::Cancelled => {
                    self.model.stop_rename();
                    self.refresh(root);
                    None
                }
                TextAction::Changed(_) | TextAction::Edited(_) | TextAction::Page { .. } | TextAction::Line { .. } => None,
            };
        }
        self.on_row_action(root, id, action.downcast_ref::<RowAction>()?)
    }
}
