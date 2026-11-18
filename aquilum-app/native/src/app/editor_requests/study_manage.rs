//! LANG-02g: explicit metadata-only rename/delete for named study groups.
//! The native host owns the SQLite path. No Markdown write or cross-note read.

use aquilum_core::learning::state::{StateError, StudyStore};
use aquilum_core::learning::state::manage::CollectionEditError;
use masonry::accesskit::Role;
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, NewWidget, PropertySet, WidgetId};
use masonry::kurbo::{Axis, Point};
use masonry::layout::Length;
use masonry::properties::types::{CrossAxisAlignment, MainAxisAlignment};
use masonry::properties::{Gap, Padding};
use masonry::widgets::Flex;

use super::super::{App, Popup};
use crate::i18n::{t, t_with};
use crate::ui::dialog::{self, CardSize, DialogAnswer, DialogIds, Modal, ConfirmDialog};
use crate::ui::editor::{TextAction, TextEditor};
use crate::ui::handle::Handle;
use crate::ui::menu::MenuChoice;
use crate::ui::search::{self, Placeholder};
use crate::ui::theme;
use crate::ui::tokens::size;
use crate::ui::widgets::{Rule, TextButton, Variant};

type ManageAction = (String, String, bool); // id, displayed name, delete?

fn store_path(app: &App) -> std::path::PathBuf {
    app.workspace.data_dir.join("language-study.sqlite")
}

fn message_for_error(error: &CollectionEditError) -> String {
    match error {
        CollectionEditError::InvalidName => t("editor.table.studyInvalidName"),
        CollectionEditError::DuplicateName => t("editor.table.studyDuplicateName"),
        CollectionEditError::MissingCollection
        | CollectionEditError::ProtectedCollection
        | CollectionEditError::State(StateError::RevisionConflict) => {
            t("editor.table.studyManageChanged")
        }
        CollectionEditError::State(_) => t("editor.table.studySaveFailed"),
    }
}

impl App {
    pub(in crate::app) fn open_study_manage(&mut self, root: &mut RenderRoot) {
        let current = StudyStore::open(&store_path(self)).and_then(|store| store.load());
        let Ok(current) = current else {
            self.study_manage_notice(root, &t("editor.table.studySaveFailed"));
            return;
        };
        let mut actions: Vec<ManageAction> = Vec::new();
        for group in current.state.registry.collections {
            if group.id != "common" {
                actions.push((group.id.clone(), group.name.clone(), false));
                actions.push((group.id, group.name, true));
            }
        }
        if actions.is_empty() {
            self.study_manage_notice(root, &t("editor.table.studyManageEmpty"));
            return;
        }
        let entries: Vec<(usize, String)> = actions.iter().enumerate()
            .map(|(i, (_, name, delete))| {
                let action = if *delete { t("editor.table.studyDelete") }
                    else { t("editor.table.studyRename") };
                (i, format!("{action}: {name}"))
            }).collect();
        let layer = self.study_menu(root, &t("editor.table.studyManageTitle"), &entries);
        self.popup = Some(Popup::StudyManage {
            layer, revision: current.revision, actions,
        });
    }

    pub(in crate::app) fn on_study_manage(
        &mut self,
        root: &mut RenderRoot,
        layer: WidgetId,
        revision: u64,
        actions: Vec<ManageAction>,
        action: &ErasedAction,
    ) {
        let Some(choice) = action.downcast_ref::<MenuChoice>() else {
            self.popup = Some(Popup::StudyManage { layer, revision, actions });
            return;
        };
        let MenuChoice::Item(index) = choice else {
            if matches!(choice, MenuChoice::Hover(_)) {
                self.popup = Some(Popup::StudyManage { layer, revision, actions });
            }
            return;
        };
        root.remove_layer(layer);
        let Some((id, name, delete)) = actions.get(*index).cloned() else { return };
        if delete {
            self.open_study_delete(root, id, name, revision);
        } else {
            self.open_study_rename(root, id, name, revision);
        }
    }

    fn open_study_rename(
        &mut self, root: &mut RenderRoot, collection_id: String,
        original: String, revision: u64,
    ) {
        let title = t("editor.table.studyRenameTitle");
        let header = dialog::header(&title, None::<NewWidget<Flex>>);
        let (entry, field, placeholder) =
            search::field(&original, t("editor.table.studyNameHint"));
        let content = NewWidget::new(
            Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch).with_fixed(entry)
        ).with_props(PropertySet::new().with(Padding::all(Length::px(size::SPACE_24))));
        let cancel = NewWidget::new(TextButton::new(t("editor.table.studyClose"), Variant::Ghost));
        let confirm = NewWidget::new(TextButton::new(t("editor.table.studyRenameSave"), Variant::Primary));
        let ids = (cancel.id(), confirm.id());
        let footer = NewWidget::new(
            Flex::row().main_axis_alignment(MainAxisAlignment::End)
                .with_fixed(cancel).with_fixed(confirm)
        ).with_props(PropertySet::new()
            .with(Gap::new(Length::px(size::DIALOG_GAP)))
            .with(Padding::all(Length::px(size::DIALOG_PADDING))));
        let border = || Rule::new(Axis::Horizontal, || theme::current().border_default);
        let card = NewWidget::new(Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_fixed(header).with_fixed(border())
            .with_fixed(content).with_fixed(border()).with_fixed(footer));
        let modal = Modal::new(card, &title, CardSize::Fit(size::CONFIRM_DIALOG_WIDTH), Role::Dialog, true);
        let dialog = DialogIds::of(&modal, ids);
        root.add_layer(modal, Point::ORIGIN);
        root.focus_on(Some(field.id()));
        self.popup = Some(Popup::StudyRename {
            dialog, field, placeholder, collection_id, revision, value: original,
        });
    }

    pub(in crate::app) fn on_study_rename(
        &mut self, root: &mut RenderRoot, id: WidgetId, dialog: DialogIds,
        field: Handle<TextEditor>, placeholder: Handle<Placeholder>,
        collection_id: String, revision: u64, mut value: String,
        action: &ErasedAction,
    ) {
        if action.downcast_ref::<DialogAnswer>() == Some(&DialogAnswer::Closed) {
            root.remove_layer(dialog.layer);
            return;
        }
        if id == field.id() {
            if let Some(TextAction::Changed(text)) = action.downcast_ref::<TextAction>() {
                value = text.clone();
                placeholder.edit(root, |mut p| Placeholder::set_empty(&mut p, value.is_empty()));
            }
            self.popup = Some(Popup::StudyRename {
                dialog, field, placeholder, collection_id, revision, value,
            });
            return;
        }
        let Some(answer) = dialog.answer(id, action) else {
            self.popup = Some(Popup::StudyRename {
                dialog, field, placeholder, collection_id, revision, value,
            });
            return;
        };
        root.remove_layer(dialog.layer);
        if answer != DialogAnswer::Confirm { return; }
        if value.trim().to_lowercase() == t("editor.table.studyDefault").to_lowercase() {
            self.study_manage_notice(root, &t("editor.table.studyDuplicateName"));
            return;
        }
        let result = StudyStore::open(&store_path(self))
            .map_err(CollectionEditError::State)
            .and_then(|mut store| store.rename_collection(revision, &collection_id, &value));
        match result {
            Ok(_) => self.study_manage_notice(root, &t_with(
                "editor.table.studyRenamed", &[("name", value.trim())],
            )),
            Err(error) => self.study_manage_notice(root, &message_for_error(&error)),
        }
    }

    fn open_study_delete(
        &mut self, root: &mut RenderRoot,
        collection_id: String, name: String, revision: u64,
    ) {
        let title = t("editor.table.studyDeleteTitle");
        let message = t_with("editor.table.studyDeleteWarning", &[("name", &name)]);
        let (widget, dialog) = ConfirmDialog::new(
            &title, &message,
            &t("editor.table.studyClose"),
            &t("editor.table.studyDeleteConfirm"),
        );
        root.add_layer(widget, Point::ORIGIN);
        root.focus_on(Some(dialog.layer));
        self.popup = Some(Popup::StudyDelete {
            dialog, collection_id, name, revision,
        });
    }

    pub(in crate::app) fn on_study_delete(
        &mut self, root: &mut RenderRoot, id: WidgetId, dialog: DialogIds,
        collection_id: String, name: String, revision: u64,
        action: &ErasedAction,
    ) {
        if action.downcast_ref::<DialogAnswer>() == Some(&DialogAnswer::Closed) {
            root.remove_layer(dialog.layer);
            return;
        }
        let Some(answer) = dialog.answer(id, action) else {
            self.popup = Some(Popup::StudyDelete {
                dialog, collection_id, name, revision,
            });
            return;
        };
        root.remove_layer(dialog.layer);
        if answer != DialogAnswer::Confirm { return; }
        let result = StudyStore::open(&store_path(self))
            .map_err(CollectionEditError::State)
            .and_then(|mut store| store.delete_collection(revision, &collection_id));
        match result {
            Ok(_) => self.study_manage_notice(root, &t_with(
                "editor.table.studyDeleted", &[("name", &name)],
            )),
            Err(error) => self.study_manage_notice(root, &message_for_error(&error)),
        }
    }

    fn study_manage_notice(&mut self, root: &mut RenderRoot, message: &str) {
        let (widget, dialog) = ConfirmDialog::new(
            &t("editor.table.studyManageTitle"), message,
            &t("editor.table.studyClose"), &t("editor.table.studyOk"),
        );
        root.add_layer(widget, Point::ORIGIN);
        root.focus_on(Some(dialog.layer));
        self.popup = Some(Popup::StudyPreview { dialog, copy: None });
    }
}
