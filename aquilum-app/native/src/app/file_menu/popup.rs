use super::*;
mod dispatch;

impl App {
    /// `DeleteNotesDialog`: заголовок и текст зависят от числа целей и от того, папка ли это.
    pub(in crate::app) fn confirm_delete(&mut self, root: &mut RenderRoot, targets: Vec<PathBuf>) {
        let many = targets.len() > 1;
        let folder = !many && targets.first().is_some_and(|p| p.is_dir());
        let name = targets.first().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let count = targets.len().to_string();
        let (title, description) = if many {
            (t("confirm.notesTitle"), t_with("confirm.manyDescription", &[("count", &count)]))
        } else if folder {
            (t("confirm.folderTitle"), t_with("confirm.folderDescription", &[("name", &name)]))
        } else {
            (t("confirm.noteTitle"), t_with("confirm.noteDescription", &[("name", &name)]))
        };
        let confirm = if many { t_with("confirm.deleteMany", &[("count", &count)]) } else { t("common.delete") };
        let (layer, dialog) = ConfirmDialog::new(&title, &description, &t("common.cancel"), &confirm);
        root.add_layer(layer, Point::ORIGIN);
        root.focus_on(Some(dialog.layer));
        self.popup = Some(Popup::Delete { dialog, targets });
    }
}
