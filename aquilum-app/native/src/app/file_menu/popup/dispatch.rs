mod dialog;

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
            Some(Popup::Recovery(prompt)) => return self.on_note_recovery_action(root, id, action, prompt),
            Some(Popup::PdfImport { dialog, source }) => match dialog.answer(id, action) {
                Some(answer) => {
                    crate::ui::dialog::close(root, dialog.layer);
                    if answer == DialogAnswer::Confirm {
                        self.start_pdf_preview(source);
                    }
                }
                None => self.popup = Some(Popup::PdfImport { dialog, source }),
            },
            Some(Popup::RenameFailed { dialog, reveal }) => self.on_rename_failed_popup(root, id, action, dialog, reveal),
            Some(Popup::PdfNotice { dialog }) => self.on_pdf_notice_popup(root, id, action, dialog),
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
            Some(Popup::TableMenu { layer, commands }) => self.on_table_menu(root, layer, commands, action), Some(Popup::StudyQuestion { layer, snapshot }) => self.on_study_question(root, layer, snapshot, action), Some(Popup::StudyAnswer { layer, snapshot, question }) => self.on_study_answer(root, layer, snapshot, question, action), Some(Popup::StudyPreview { dialog, copy }) => self.on_study_preview(root, id, dialog, copy, action), Some(Popup::StudyAddQuestion { layer, pending }) => self.on_study_add_question(root, layer, pending, action), Some(Popup::StudyAddAnswer { layer, pending, question }) => self.on_study_add_answer(root, layer, pending, question, action), Some(Popup::StudyAddCollection { layer, pending, choices }) => self.on_study_add_collection(root, layer, pending, choices, action), Some(Popup::StudyNewCollection { dialog, field, placeholder, pending, value }) => self.on_study_new_collection(root, id, dialog, field, placeholder, pending, value, action), Some(Popup::StudyManage { layer, revision, actions }) => self.on_study_manage(root, layer, revision, actions, action), Some(Popup::StudyRename { dialog, field, placeholder, collection_id, revision, value }) => self.on_study_rename(root, id, dialog, field, placeholder, collection_id, revision, value, action), Some(Popup::StudyDelete { dialog, collection_id, name, revision }) => self.on_study_delete(root, id, dialog, collection_id, name, revision, action), Some(Popup::StudyPicker { layer, choices, selected }) => self.on_study_picker(root, layer, choices, selected, action), Some(Popup::StudyCard { dialog, snapshot, index, revealed }) => self.on_study_card(root, id, dialog, snapshot, index, revealed, action), Some(Popup::StudyChoice { layer, snapshot, index, question, progress }) => self.on_study_choice(root, layer, snapshot, index, question, progress, action), Some(Popup::StudyChoiceResult { dialog, snapshot, index, progress }) => self.on_study_choice_result(root, id, dialog, snapshot, index, progress, action), Some(Popup::StudyMatchQuestion { layer, state }) => self.on_study_match_question(root, layer, state, action), Some(Popup::StudyMatchAnswer { layer, state, question_index }) => self.on_study_match_answer(root, layer, state, question_index, action), Some(Popup::StudyMatchFeedback { dialog, state }) => self.on_study_match_feedback(root, id, dialog, state, action), Some(Popup::StudyTypedEntry { dialog, field, placeholder, snapshot, index, progress, value }) => self.on_study_typed(root, id, dialog, field, placeholder, snapshot, index, progress, value, action), Some(Popup::StudyTypedResult { dialog, snapshot, index, progress }) => self.on_study_typed_result(root, id, dialog, snapshot, index, progress, action),
            Some(Popup::EditorMenu { layer, commands }) => self.on_editor_menu(root, layer, commands, action), Some(Popup::AnnotationColors { layer, pending }) => self.on_annotation_colors(root, layer, pending, action), Some(Popup::AnnotationExplanation { dialog, field, placeholder, pending, value }) => self.on_annotation_explanation(root, id, dialog, field, placeholder, pending, value, action),
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
            Some(popup @ (Popup::Settings { .. } | Popup::Search(_) | Popup::Transcript(_) | Popup::Templates(_) | Popup::Workspaces(_) | Popup::Reader(_) | Popup::Transfers(_))) => self.popup = Some(popup),
            None => {}
        }
        Outcome::default()
    }
}
