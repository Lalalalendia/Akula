mod worker;

use std::sync::Arc;

use masonry::app::RenderRoot;

use super::FileTree;

impl FileTree {
    pub fn reload(&mut self, root: &mut RenderRoot) {
        self.model.reload_all();
        self.count_notes(Arc::clone(&self.wake));
        self.refresh(root);
    }

    pub fn poll(&mut self, root: &mut RenderRoot) {
        let Some(counts) = self.counting.as_ref().and_then(|receiver| receiver.try_recv().ok()) else { return };
        self.counting = None;
        if self.pending_count_scan {
            self.pending_count_scan = false;
            self.count_notes(Arc::clone(&self.wake));
            return;
        }
        self.model.set_counts(counts);
        self.refresh(root);
    }
}
