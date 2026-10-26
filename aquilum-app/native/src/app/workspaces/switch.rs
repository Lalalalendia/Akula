use super::*;

impl App {
    pub(in crate::app) fn switch_workspace(&mut self, root: &mut RenderRoot, path: PathBuf) -> Outcome {
        let path = crate::workspace::normalize(&path);
        let reopen = matches!(self.popup, Some(Popup::Workspaces(_)));
        let failed = |app: &mut App, root: &mut RenderRoot| {
            let error = t_with("workspaces.openFailed", &[("path", &path.to_string_lossy())]);
            if reopen || app.tree.root().is_some() {
                app.close_workspaces(root);
                app.show_workspaces(root, Some(error));
            } else {
                app.vault_error = Some(error);
                app.replace_root(root);
                app.show_active(root);
            }
            Outcome::default()
        };
        if !path.is_dir() {
            return failed(self, root);
        }
        if self.tree.root().is_some_and(|current| workspaces::same_path(current, &path)) {
            self.close_workspaces(root);
            return Outcome::default();
        }
        if !self.save_before_workspace_change(root) {
            self.close_workspaces(root);
            self.show_workspaces(root, Some(t("workspaces.switchSaveFailed")));
            return Outcome::default();
        }
        // Do not remember the target as the last opened folder until the old
        // note is safely saved; a refused switch must survive an app restart.
        if let Err(e) = self.workspace.core.ui_state.resolve_workspace(&path.to_string_lossy(), crate::dates::now_ms()) {
            report!("база {} не открыта: {e:?}", path.display());
            return failed(self, root);
        }
        self.close_workspaces(root);
        self.leave_note();
        // No document content from the previous workspace survives the switch,
        // even if the same relative pathname exists in the next vault.
        self.inactive_notes.clear();
        if let Some(session) = &mut self.session {
            session.save(&self.workspace.core, &mut self.tabs);
        }
        self.workspace.root = Some(path.clone());
        self.tree = FileTree::new(Some(path.clone()), &self.workspace.data_dir, std::sync::Arc::clone(&self.workspace.wake));
        self.navigation = crate::navigation::Navigation::load(&self.workspace.data_dir, Some(&path));
        (self.session, self.tabs) = match Session::open(&self.workspace.core, &path) {
            Some((session, tabs)) => (Some(session), tabs),
            None => (None, Tabs::default()),
        };
        self.text = crop::Rope::new();
        self.vault_error = None;
        self.renaming_tab = None;
        self.rename_field = None;
        self.listing = None;
        self.panel_body = None;
        self.history = None;
        self.history_body = None;
        self.opened_versions = HashMap::new();
        self.version_view = None;
        self.version_notice = None;
        self.home = None;
        self.graph = super::super::graph_page::GraphState::default();
        self.start_index();
        self.refresh_home();
        self.cleanup_retention();
        self.replace_root(root);
        self.show_active(root);
        Outcome { title: true, ..Outcome::default() }
    }
}
