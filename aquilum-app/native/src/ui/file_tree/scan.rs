pub(super) mod worker;

use std::sync::Arc;

use masonry::app::RenderRoot;

use super::FileTree;

impl FileTree {
    pub fn reload(&mut self, root: &mut RenderRoot) {
        self.model.reload_all();
        self.count_notes(Arc::clone(&self.wake));
        #[cfg(all(windows, feature = "p2p-inbox-preview"))]
        self.request_imported();
        self.refresh(root);
    }

    pub(super) fn count_notes(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        if let Some(path) = self.model.root() {
            self.counting.request(path.to_path_buf(), wake);
        }
    }

    pub fn poll(&mut self, root: &mut RenderRoot) {
        let Some(path) = self.model.root() else { return };
        let mut changed = false;
        if let Some(counts) = self.counting.poll(path, Arc::clone(&self.wake)) {
            self.model.set_counts(counts);
            changed = true;
        }
        #[cfg(all(windows, feature = "p2p-inbox-preview"))]
        {
            changed |= self.poll_imported();
        }
        if changed {
            self.refresh(root);
        }
    }
}
