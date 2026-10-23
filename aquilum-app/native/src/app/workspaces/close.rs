use super::*;

impl App {
    // Return to the no-vault screen without forgetting the known workspace or deleting files.
    pub(in crate::app) fn close_current_workspace(&mut self, root: &mut RenderRoot) -> Outcome {
        if self.tree.root().is_none() {
            return Outcome::default();
        }
        // Unlike an ordinary tab switch, explicitly closing a workspace must not
        // discard an unsaved conflict or a failed write from the active note.
        if let Some(note) = self.note.as_mut() {
            note.flush(&self.workspace.core, &self.text);
            note.tick(&self.workspace.core, &self.text, std::time::Instant::now());
            if note.save_error.is_some() {
                self.refresh_save_error(root);
                report!("база не закрыта: сохранение заметки требует проверки");
                return Outcome::default();
            }
        }
        if let Err(error) = self.workspace.core.search.deactivate() {
            report!("база не закрыта: поисковый индекс не остановлен: {error:?}");
            return Outcome::default();
        }
        self.leave_note();
        if let Some(session) = &mut self.session {
            session.save(&self.workspace.core, &mut self.tabs);
        }
        // Stop the old folder watcher before entering no-vault mode.
        self.workspace.core.watcher.stop();
        self.workspace.root = None;
        self.tree = FileTree::new(None, &self.workspace.data_dir, std::sync::Arc::clone(&self.workspace.wake));
        self.navigation = crate::navigation::Navigation::load(&self.workspace.data_dir, None);
        self.session = None;
        self.tabs = Tabs::default();
        self.note = None;
        self.note_ids = None;
        self.text = crop::Rope::new();
        self.inactive_notes.clear();
        self.find = None;
        self.renaming_tab = None;
        self.rename_field = None;
        self.renaming_to = None;
        self.open_error = None;
        self.open_error_ids = None;
        self.save_error_ids = None;
        self.vault_error = None;
        self.pending_reveal = None;
        self.listing = None;
        self.panel_key = None;
        self.panel_body = None;
        self.history = None;
        self.history_body = None;
        self.opened_versions = HashMap::new();
        self.version_view = None;
        self.version_notice = None;
        self.home = None;
        self.graph = super::super::graph_page::GraphState::default();
        self.workspace.core.active_note.set(None);
        self.refresh_home();
        self.replace_root(root);
        self.show_active(root);
        Outcome { title: true, ..Outcome::default() }
    }
}
