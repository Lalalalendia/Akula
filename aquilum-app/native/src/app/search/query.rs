use masonry::app::RenderRoot;

use super::{App, Popup};

impl App {
    pub(in crate::app) fn on_search_results(&mut self, root: &mut RenderRoot) {
        let Some(Popup::Search(state)) = &mut self.popup else {
            let _ = self.search_rx.try_iter().count();
            return;
        };
        let (worker, current) = (state.worker.id, state.worker.current());
        let mut delivered = false;
        for found in self.search_rx.try_iter().filter(|f| f.worker == worker && f.generation == current) {
            delivered = true;
            state.apply(found.results);
        }
        if delivered { self.render_search(root); }
    }

    pub(super) fn query_changed(&mut self, root: &mut RenderRoot, query: String) {
        let workspace = self.tree.root().map(|p| p.to_path_buf());
        let Some(Popup::Search(state)) = &mut self.popup else { return };
        state.query = query;
        let trimmed = state.query.trim();
        if trimmed.is_empty() {
            state.worker.stop();
            state.results.clear();
            state.speech.clear();
            state.terms.clear();
            self.render_search(root);
        } else {
            state.worker.ask(workspace, trimmed.to_owned());
            self.show_placeholder(root);
        }
    }
}
