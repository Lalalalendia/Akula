use super::*;

impl App {
    pub(crate) fn open_in_with_offset(&mut self, root: &mut RenderRoot, path: PathBuf, new_tab: bool, offset: Option<usize>) -> bool {
        let image = crate::images::kind(&path.to_string_lossy()) == Some(crate::images::MediaKind::Image);
        #[cfg(feature = "dicom-preview")]
        let dicom = crate::dicom::is_candidate(&path) || self.dicom_external_sources.contains(&path);
        #[cfg(not(feature = "dicom-preview"))]
        let dicom = false;
        if !image && !dicom && !is_markdown(&path) && !is_plain_text(&path) && !is_structured_file(&path) && !crate::docx::is_docx_path(&path) {
            return false;
        }
        // Validate the image before changing tabs or navigation: no paths
        // outside this vault, symlinks, oversized or corrupt sources.
        if image && self.tree.root().and_then(|vault| crate::app::media::local_image_preview(vault, &path)).is_none() {
            return false;
        }
        if crate::docx::is_docx_path(&path) && self.tree.root().and_then(|v| crate::docx::admit(v, &path).ok()).is_none() { return false; }
        #[cfg(feature = "dicom-preview")]
        if dicom && !self.is_dicom_admitted(&path) { return false; }
        if let Some(current) = self.note.as_ref().map(|n| n.path.clone()) {
            self.navigation.push(&current);
        }
        self.navigation.push(&path);
        self.pending_reveal = offset;
        if new_tab {
            self.tabs.open_new(path);
        } else {
            self.tabs.open(path);
        }
        self.show_active(root);
        true
    }

}
