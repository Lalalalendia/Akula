//! Lightweight read-only DOCX tab; never constructs an editable Note.
use super::*;

impl App {
    pub(super) fn show_docx_tab(&mut self, root: &mut RenderRoot, path: std::path::PathBuf) {
        self.leave_note();
        self.drop_note_page(root);
        self.show_docx_readonly(root, &path);
        self.tree.set_active(root, Some(path));
        self.show_page(root, 0);
    }
    pub(super) fn show_docx_readonly(&mut self, root: &mut RenderRoot, path: &Path) {
        self.reset_note_view_for_preview();

        let result = self.tree.root().ok_or_else(|| "Не открыта база Akula".to_owned())
            .and_then(|vault| crate::docx::read(vault, path));
        match result {
            Ok(text) => {
                let filename = path.file_name().unwrap_or_default().to_string_lossy();
                let page = crate::app::ui_actions::readonly_text_page(&format!("{filename} — DOCX, только чтение"), &text);
                if let Some(chrome) = &self.chrome {
                    chrome.note_page.edit(root, |mut slot| Slot::set_child(&mut slot, page.erased()));
                }
            }
            Err(error) => {
                self.open_error = Some((path.to_path_buf(), error.clone()));
                self.show_open_error(root, path, &error);
            },
        }
    }
}
