use super::*;
mod dispatch;

impl App {
    pub fn new_note(&mut self, root: &mut RenderRoot) -> bool {
        self.create_note_titled(root, None)
    }

    pub(super) fn create_note_titled(&mut self, root: &mut RenderRoot, title: Option<&str>) -> bool {
        let Some(workspace) = self.tree.root().map(Path::to_path_buf) else { return false };
        match file_ops::create_note(&self.workspace.core, &workspace, title) {
            Ok(path) => {
                self.tree.paths_changed(root, std::slice::from_ref(&path));
                self.open(root, path)
            }
            Err(error) => {
                report!("заметка не создана: {error}");
                false
            }
        }
    }

    pub(super) fn create_note_in(&mut self, root: &mut RenderRoot, folder: &Path) -> bool {
        match file_ops::create_note(&self.workspace.core, folder, None) {
            Ok(path) => {
                self.tree.paths_changed(root, std::slice::from_ref(&path));
                self.tree.reveal(root, &path);
                let opened = self.open(root, path.clone());
                self.tree.start_rename(root, path);
                opened
            }
            Err(error) => {
                report!("заметка не создана: {error}");
                false
            }
        }
    }

    /// Действие в основном окне; `true` — сменилась открытая заметка (и заголовок окна).
    pub(super) fn on_main_action(&mut self, root: &mut RenderRoot, id: WidgetId, action: &ErasedAction, window: Size) -> bool {
        match self.tree.on_action(root, id, action) {
            Some(TreeAction::Open { path, new_tab }) => return self.open_tree_file(root, path, new_tab),
            Some(TreeAction::Rename { from, to }) => {
                self.rename_file(root, from, to);
                return false;
            }
            Some(TreeAction::CreateNote) => return self.create_note_titled(root, None),
            Some(TreeAction::NewNote { folder }) => return self.create_note_in(root, &folder),
            Some(TreeAction::NewFolder { folder }) => {
                match file_ops::create_folder(&self.workspace.core, &folder) {
                    Ok(path) => {
                        self.tree.paths_changed(root, std::slice::from_ref(&path));
                        self.tree.reveal(root, &path);
                        self.tree.start_rename(root, path);
                    }
                    Err(error) => report!("папка не создана: {error}"),
                }
                return false;
            }
            Some(TreeAction::SortMenu { button }) => {
                self.open_sort_menu(root, button);
                return false;
            }
            Some(TreeAction::Move { sources, target }) => {
                file_ops::move_all(&self.workspace.core, sources, target);
                return false;
            }
            Some(TreeAction::ContextMenu { targets, clicked, folder, at }) => {
                self.open_file_menu(root, targets, clicked, folder, at, window, None);
                return false;
            }
            None => {}
        }
        let Some(chrome) = &self.chrome else { return false };
        if id == chrome.toggle_sidebar {
            let open = !self.interface.sidebar_open();
            self.interface.set_sidebar_open(open);
            chrome.sidebar.edit(root, |mut sidebar| crate::ui::widgets::Collapse::set_open(&mut sidebar, open));
            root.edit_widget(id, |mut w| crate::ui::widgets::IconButton::set_label(&mut w.downcast(), &crate::i18n::t(crate::ui::sidebar_label(open))));
            self.shown = Some(self.visible_panels());
            self.apply_widths(root);
        } else if (id == chrome.settings || id == chrome.rail_settings) && action.downcast_ref::<Pressed>().is_some() {
            self.open_settings(root);
        } else if id == chrome.search_button && action.downcast_ref::<Pressed>().is_some() {
            self.open_search(root);
        } else if id == chrome.tab_list && action.downcast_ref::<Pressed>().is_some() {
            self.open_tabs_menu(root, id);
        } else if Some(id) == chrome.graph_button && action.downcast_ref::<Pressed>().is_some() {
            self.open_graph(root);
            return true;
        } else if id == chrome.transfer_button && action.downcast_ref::<Pressed>().is_some() {
            self.open_transfers(root);
        } else if Some(id) == chrome.home_button
            && action.downcast_ref::<Pressed>().is_some()
            && let Some(home) = self.home.clone()
        {
            return self.open_in(root, home, true);
        }
        false
    }
}
