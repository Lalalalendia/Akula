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
                #[cfg(feature = "dicom-preview")]
                let protect = readonly_dicom_target(&clicked, &targets);
                #[cfg(not(feature = "dicom-preview"))]
                let protect = false;
                match choice {
                    MenuChoice::Sub(index) => {
                        let folders: Vec<PathBuf> = targets.into_iter().filter(|path| path.is_dir()).collect();
                        self.tree.set_color(root, &folders, index.checked_sub(1).and_then(|at| palette::NAMES.get(at).copied()));
                    }
                    #[cfg(feature = "dicom-preview")]
                    MenuChoice::Item(MENU_DICOM_OPEN) if tab.is_none() && targets.len() == 1 && clicked.is_dir() => {
                        if self.open_dicom_external(root, clicked) {
                            return Outcome { title: true, ..Outcome::default() };
                        }
                    }
                    #[cfg(feature = "dicom-preview")]
                    MenuChoice::Item(MENU_DICOM_PICK_FOLDER) if tab.is_none() && clicked.is_dir() => {
                        self.pick_dicom_source(true);
                    }
                    #[cfg(feature = "dicom-preview")]
                    MenuChoice::Item(MENU_DICOM_PICK_FILE) if tab.is_none() && clicked.is_dir() => {
                        self.pick_dicom_source(false);
                    }
                    #[cfg(feature = "dicom-preview")]
                    MenuChoice::Item(MENU_DICOM_FILE) if tab.is_none() && !clicked.is_dir() && crate::dicom::is_candidate(&clicked) => {
                        if self.open_dicom_external(root, clicked) {
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
                    MenuChoice::Item(MENU_DUPLICATE) if !protect => file_ops::duplicate_all(&self.workspace.core, targets),
                    MenuChoice::Item(MENU_DELETE) if !protect => self.confirm_delete(root, targets),
                    MenuChoice::Item(MENU_CLOSE) if let Some(tab) = tab => {
                        self.tabs.close(tab);
                        self.show_active(root);
                        return Outcome { title: true, ..Outcome::default() };
                    }
                    MenuChoice::Item(MENU_RENAME) if tab.is_some() && !protect => self.start_tab_rename(root, &clicked),
                    MenuChoice::Item(MENU_RENAME) if !protect => self.tree.start_rename(root, clicked),
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

}
