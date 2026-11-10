use std::path::Path;
use std::sync::{Arc, mpsc};
use masonry::app::RenderRoot;
use super::super::{FileTree, model};

impl FileTree {
    /// The scan uses the same vetted directory reader as the tree: symlinks
    /// and internal data folders never enter the background index.
    pub(in crate::ui::file_tree) fn reindex(&mut self) {
        let Some(root) = self.model.root().filter(|_| self.model.filtering()).map(Path::to_path_buf) else {
            self.indexing = None;
            return;
        };
        let wake = Arc::clone(&self.wake);
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            if sender.send(model::index(&root)).is_ok() {
                wake();
            }
        });
        self.indexing = Some(receiver);
    }

    pub(super) fn poll_index(&mut self, root: &mut RenderRoot) {
        let Some(index) = self.indexing.as_ref().and_then(|receiver| receiver.try_recv().ok()) else { return };
        self.indexing = None;
        self.model.set_index(index);
        self.refresh(root);
    }
}
