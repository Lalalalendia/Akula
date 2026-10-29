mod rename;

use std::path::{Path, PathBuf};

use aquilum_core::search::paths::same_path;
use masonry::app::RenderRoot;

use super::super::App;

impl App {
    pub(in crate::app) fn close_missing_tabs(&mut self, root: &mut RenderRoot, removed: &[PathBuf]) -> bool {
        let inside = |path: &Path| path.ancestors().any(|ancestor| removed.iter().any(|gone| same_path(ancestor, gone)));
        let gone: Vec<_> = self.tabs.tabs().iter().filter(|tab| tab.path.as_ref().is_some_and(|p| !p.exists() && inside(p))).map(|tab| tab.id).collect();
        for id in &gone {
            self.tabs.close(*id);
        }
        if !gone.is_empty() {
            self.show_active(root);
        }
        !gone.is_empty()
    }
}
