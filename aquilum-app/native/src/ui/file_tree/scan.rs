pub(super) mod worker;

use std::sync::Arc;

use masonry::app::RenderRoot;

use super::FileTree;

impl FileTree {
    pub fn reload(&mut self, root: &mut RenderRoot) {
        self.model.reload_all();
        self.count_notes(Arc::clone(&self.wake));
        self.refresh(root);
    }

    pub(super) fn count_notes(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        if let Some(path) = self.model.root() {
            self.counting.request(path.to_path_buf(), wake);
        }
    }

    pub fn poll(&mut self, root: &mut RenderRoot) {
        let Some(path) = self.model.root() else { return };
        let Some(counts) = self.counting.poll(path, Arc::clone(&self.wake)) else { return };
        self.model.set_counts(counts);
        self.refresh(root);
    }
}
