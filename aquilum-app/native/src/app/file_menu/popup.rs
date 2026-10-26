use super::*;

impl App {
    pub(in crate::app) fn on_popup_action(&mut self, root: &mut RenderRoot, id: WidgetId, action: &ErasedAction) -> Outcome {
        match self.popup.take() {
            Some(Popup::FileMenu { layer, targets, clicked, tab }) => {
                let Some(choice) = action.downcast_ref::<MenuChoice>() else {
                    self.popup = Some(Popup::FileMenu { layer, targets, clicked, tab });
                    return Outcome::default();
                };
                if let MenuChoice::Hover(index) = choice {
                    root.edit_widget(layer, |mut menu| Menu::show_sub(&mut menu.downcast(), clicked.is_dir() && *index == MENU_COLOR));
                    self.popup = Some(Popup::FileMenu { layer, targets, clicked, tab });
                    return Outcome::default();
                }
                if let MenuChoice::Item(_) | MenuChoice::Sub(_) = choice {
                    root.remove_layer(layer);
                }
                match choice {
                    MenuChoice::Sub(index) => {
                        let folders: Vec<PathBuf> = targets.into_iter().filter(|path| path.is_dir()).collect();
                        self.tree.set_color(root, &folders, index.checked_sub(1).and_then(|at| palette::NAMES.get(at).copied()));
                    }
                    #[cfg(feature = "dicom-preview")]
                    MenuChoice::Item(MENU_DICOM_OPEN) if tab.is_none() && targets.len() == 1 && clicked.is_dir() && clicked.join("DICOMDIR").is_file() => {
                        if self.open_in(root, clicked.join("DICOMDIR"), true) {
                            return Outcome { title: true, ..Outcome::default() };
                        }
                    }
                    MenuChoice::Item(MENU_PDF_PICK) if tab.is_none() && clicked.is_dir() => {
                        self.pick_pdf_file();
                    }
                    MenuChoice::Item(MENU_PDF_PREVIEW) if tab.is_none()
                        && targets.len() == 1 && pdf_preview::is_pdf_path(&clicked) => {
                        self.prompt_pdf_preview(root, clicked);
                    }
                    MenuChoice::Item(MENU_DUPLICATE) => file_ops::duplicate_all(&self.workspace.core, targets),
                    MenuChoice::Item(MENU_DELETE) => self.confirm_delete(root, targets),
                    MenuChoice::Item(MENU_CLOSE) if let Some(tab) = tab => {
                        self.tabs.close(tab);
                        self.show_active(root);
                        return Outcome { title: true, ..Outcome::default() };
                    }
                    MenuChoice::Item(MENU_RENAME) if tab.is_some() => self.start_tab_rename(root, &clicked),
                    MenuChoice::Item(MENU_RENAME) => self.tree.start_rename(root, clicked),
                    MenuChoice::Item(_) | MenuChoice::Dismissed | MenuChoice::Hover(_) => {}
                }
            }
            Some(Popup::PdfImport { dialog, source }) => match dialog.answer(id, action) {
                Some(answer) => {
                    crate::ui::dialog::close(root, dialog.layer);
                    if answer == DialogAnswer::Confirm {
                        self.start_pdf_preview(source);
                    }
                }
                None => self.popup = Some(Popup::PdfImport { dialog, source }),
            },
            Some(Popup::PdfNotice { dialog }) => match dialog.answer(id, action) {
                Some(_) => crate::ui::dialog::close(root, dialog.layer),
                None => self.popup = Some(Popup::PdfNotice { dialog }),
            },
            Some(Popup::Delete { dialog, targets }) => match dialog.answer(id, action) {
                Some(answer) => {
                    crate::ui::dialog::close(root, dialog.layer);
                    if answer == DialogAnswer::Confirm
                        && let Some(workspace) = self.tree.root().map(Path::to_path_buf)
                    {
                        file_ops::trash_all(&self.workspace.core, workspace, targets);
                        self.tree.clear_selection(root);
                    }
                }
                None => self.popup = Some(Popup::Delete { dialog, targets }),
            },
            Some(Popup::VersionMenu { layer, index, commands }) => self.on_version_menu(root, layer, index, commands, action),
            Some(Popup::TableMenu { layer, commands }) => self.on_table_menu(root, layer, commands, action),
            Some(Popup::EditorMenu { layer, commands }) => self.on_editor_menu(root, layer, commands, action),
            Some(Popup::NoteActionsMenu { layer }) => return self.on_note_actions_menu(root, layer, action),
            Some(Popup::SortMenu { layer, button }) => match self.dropdown_answer(root, layer, button, id, action) {
                None => self.popup = Some(Popup::SortMenu { layer, button }),
                Some(chosen) => {
                    if let Some(sort) = chosen.and_then(|index| crate::ui::file_tree::Sort::ALL.get(index)) {
                        self.tree.set_sort(root, *sort);
                    }
                }
            },
            Some(Popup::TabsMenu { layer, button }) => match self.dropdown_answer(root, layer, button, id, action) {
                None => self.popup = Some(Popup::TabsMenu { layer, button }),
                Some(chosen) => return chosen.map_or_else(Outcome::default, |index| self.pick_tab(root, index)),
            },
            Some(Popup::Viewer { layer }) => {
                if action.downcast_ref::<crate::ui::viewer::ViewerClosed>().is_some() {
                    root.remove_layer(layer);
                } else {
                    self.popup = Some(Popup::Viewer { layer });
                }
            }
            Some(popup @ (Popup::Settings { .. } | Popup::Search(_) | Popup::Templates(_) | Popup::Workspaces(_) | Popup::Reader(_) | Popup::Transfers(_))) => self.popup = Some(popup),
            None => {}
        }
        Outcome::default()
    }

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
