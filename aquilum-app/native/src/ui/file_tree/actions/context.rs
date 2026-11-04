use super::*;

impl FileTree {
    pub(super) fn context_menu(&mut self, root: &mut RenderRoot, index: usize, at: Point) -> Option<TreeAction> {
        let row = self.model.rows()[index].clone();
        let note = aquilum_core::search::paths::is_markdown(&row.path) || aquilum_core::search::paths::is_plain_text(&row.path);
        #[cfg(feature = "dicom-preview")]
        let dicom = crate::dicom::is_candidate(&row.path);
        #[cfg(not(feature = "dicom-preview"))]
        let dicom = false;
        if !row.folder && !note && !dicom && !row.path.to_string_lossy().to_ascii_lowercase().ends_with(".pdf") {
            return None;
        }
        let targets = self.model.focus(index);
        self.refresh(root);
        Some(TreeAction::ContextMenu { targets, clicked: row.path, folder: row.folder, at })
    }

}
