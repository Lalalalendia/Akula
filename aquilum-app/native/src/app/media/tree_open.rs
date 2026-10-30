use std::path::PathBuf;
use masonry::app::RenderRoot;
use crate::app::App;

impl App {
    // File-tree images open in a normal tab. Embedded images may still use the modal viewer.
    pub(in crate::app) fn open_tree_file(&mut self, root: &mut RenderRoot, path: PathBuf, new_tab: bool) -> bool {
        let opened = self.open_in(root, path.clone(), new_tab);
        // UI-only diagnostic for Win7 QA: an accepted route is not proof
        // that Note::open decoded the file successfully.
        #[cfg(target_os = "windows")]
        if crate::host::native_frame() && aquilum_core::search::paths::is_plain_text(&path) {
            report!(
                "WIN7_TXT_TREE_OPEN routed={} decoded={} file_exists={}",
                opened,
                self.open_error.is_none() && self.tabs.active().path.as_deref() == Some(path.as_path()) && (self.note.is_none() || self.note.as_ref().is_some_and(|note| note.is(&path))),
                path.is_file()
            );
        }
        opened
    }
}
