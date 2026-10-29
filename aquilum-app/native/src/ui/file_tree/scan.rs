pub(super) mod worker;
mod poll;

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

}
