//! Native read-only DICOM tab, opt-in under the dicom-preview feature.
use super::*;

impl App {
    pub(super) fn show_dicom_tab(&mut self, root: &mut RenderRoot, path: PathBuf) {
        self.leave_note();
        self.drop_note_page(root);
        self.show_dicom_readonly(root, &path);
        let external = self.dicom_external_sources.contains(&path);
        self.tree.set_active(root, if external { None } else { Some(path) });
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
        let result = self.dicom_scope(path)
            .ok_or_else(|| "DICOM: необходимо явно выбрать внешний файл или папку".to_owned())
            .and_then(|allowed_root| crate::dicom::admit(&allowed_root, path)
                .map(|source| crate::ui::dicom::DicomPage::new(allowed_root, source)));
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
    /// Explicit, ephemeral read-only permission for the exact chosen source.
    /// No workspace mutation, file copying, or extraction is performed.
    pub(super) fn dicom_scope(&self, path: &Path) -> Option<PathBuf> {
        if self.dicom_external_sources.contains(path) {
            if path.is_dir() { return Some(path.to_path_buf()); }
            return path.parent().map(Path::to_path_buf);
        }
        self.tree.root().map(Path::to_path_buf)
    }

    pub(super) fn is_dicom_admitted(&self, path: &Path) -> bool {
        self.dicom_scope(path)
            .is_some_and(|root| crate::dicom::admit(&root, path).is_ok())
    }

    pub(super) fn open_dicom_external(&mut self, root: &mut RenderRoot, path: PathBuf) -> bool {
        if !path.is_dir() && !crate::dicom::is_candidate(&path) {
            self.show_open_error(root, &path, "DICOM: выберите папку, ZIP или файл DICOM");
            return false;
        }
        let scope = if path.is_dir() { path.clone() }
                    else if let Some(parent) = path.parent() { parent.to_path_buf() }
                    else { return false };
        let Ok(source) = crate::dicom::admit(&scope, &path) else {
            self.show_open_error(root, &path, "DICOM: выбранный источник недоступен или небезопасен");
            return false;
        };
        // Explicit permission for this process lifetime. Do not persist it to
        // workspace settings and do not grant access to sibling files.
        self.dicom_external_sources.insert(source.clone());
        if self.open_in(root, source.clone(), true) { return true; }
        self.dicom_external_sources.remove(&source);
        false
    }

    pub(super) fn pick_dicom_source(&self, folder: bool) {
        let sender = self.picked_tx.clone();
        let wake = std::sync::Arc::clone(&self.workspace.wake);
        std::thread::spawn(move || {
            let dialog = rfd::FileDialog::new();
            let picked = if folder { dialog.pick_folder() } else { dialog.pick_file() };
            if let Some(path) = picked
                && sender.send(super::super::book::Picked::DicomExternal(path)).is_ok() {
                wake();
            }
        });
    }

}
