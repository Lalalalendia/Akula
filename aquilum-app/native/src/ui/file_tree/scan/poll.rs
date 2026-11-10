use super::*;
impl FileTree {
    pub fn poll(&mut self, root: &mut RenderRoot) {
        self.poll_index(root);
        let Some(path) = self.model.root() else { return };
        let Some(counts) = self.counting.poll(path, Arc::clone(&self.wake)) else { return };
        self.model.set_counts(counts);
        self.refresh(root);
    }
}
