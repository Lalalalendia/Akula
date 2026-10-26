//! Native read-only DICOM tab, opt-in under the dicom-preview feature.
use super::*;

impl App {
    pub(super) fn show_dicom_tab(&mut self, root: &mut RenderRoot, path: PathBuf) {
        self.leave_note();
        self.drop_note_page(root);
        self.show_dicom_readonly(root, &path);
        self.tree.set_active(root, Some(path));
        self.show_page(root, 0);
    }

    pub(super) fn show_dicom_readonly(&mut self, root: &mut RenderRoot, path: &Path) {
        self.note = None;
        self.note_ids = None;
        self.embeds = None;
        self.open_error = None;
        self.open_error_ids = None;
        // Do admission on the caller, but keep expensive DICOMDIR scans and
        // frame decoding in a bounded worker instead of blocking Masonry.
        let result = self.tree.root()
            .ok_or_else(|| "Не открыта база Akula".to_owned())
            .and_then(|vault| {
                crate::dicom::admit(vault, path)
                    .map(|source| crate::ui::dicom::DicomPage::new(vault.to_path_buf(), source))
            });
        match result {
            Ok(page) => {
                let id = page.id();
                if let Some(chrome) = &self.chrome {
                    chrome.note_page.edit(root, |mut slot| Slot::set_child(&mut slot, page.erased()));
                }
                root.focus_on(Some(id));
            }
            Err(error) => {
                self.open_error = Some((path.to_path_buf(), error.clone()));
                self.show_open_error(root, path, &error);
            }
        }
    }
}
