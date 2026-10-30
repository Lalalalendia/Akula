use std::sync::Arc;

use masonry::app::RenderRoot;

use super::super::FileTree;

impl FileTree {
    pub fn poll(&mut self, root: &mut RenderRoot) {
        let Some(path) = self.model.root() else {
            return;
        };
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
