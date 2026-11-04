use super::{FileTree, Path, PathBuf, RenderRoot};

impl FileTree {
    /// Patch names, expanded paths and colors only after an actual disk rename.
    pub fn rename_succeeded(&mut self, root: &mut RenderRoot, from: &Path, to: &Path) {
        let name = if aquilum_core::search::paths::is_markdown(to) {
            to.file_stem()
        } else {
            to.file_name()
        }.unwrap_or_default().to_string_lossy().into_owned();
        self.model.patch(from, to, &name);
        self.save_expanded();
        self.save_colors();
        if to.is_dir() {
            self.reload(root);
        } else {
            self.paths_changed(root, &[from.to_path_buf(), to.to_path_buf()]);
        }
    }

    pub fn paths_changed(&mut self, root: &mut RenderRoot, paths: &[PathBuf]) {
        self.model.refresh_paths(paths);
        self.refresh(root);
    }
}
