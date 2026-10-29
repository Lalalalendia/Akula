use std::path::{Path, PathBuf};

use masonry::app::RenderRoot;

use super::{FileTree, Sort};

impl FileTree {
    pub fn sort(&self) -> Sort {
        self.model.sort()
    }

    pub fn set_sort(&mut self, root: &mut RenderRoot, sort: Sort) {
        if !self.model.set_sort(sort) {
            return;
        }
        if let Some(workspace) = self.model.root().map(Path::to_path_buf) {
            self.store.set_sort(&workspace, sort);
        }
        self.refresh(root);
    }

    pub fn color(&self, path: &Path) -> Option<&'static str> {
        self.model.color(path)
    }

    pub fn set_color(&mut self, root: &mut RenderRoot, folders: &[PathBuf], color: Option<&'static str>) {
        self.model.set_color(folders, color);
        self.save_colors();
        self.refresh(root);
    }

    pub(super) fn save_colors(&mut self) {
        if let Some(root) = self.model.root().map(Path::to_path_buf) {
            self.store.set_colors(&root, &self.model.color_entries());
        }
    }

    pub(super) fn save_expanded(&mut self) {
        if let Some(root) = self.model.root().map(Path::to_path_buf) {
            self.store.set(&root, &self.model.expanded_paths());
        }
    }
}
