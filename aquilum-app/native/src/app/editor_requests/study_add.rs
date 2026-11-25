//! First-party opt-in saving from an authorized live editor to the atomic
//! language metadata store. No Markdown writes, hidden file reads or plugins.

use std::path::PathBuf;
use aquilum_core::learning::{ColumnMapping, editor_snapshot::EditorTableSnapshot};
use aquilum_core::learning::table_anchors::NoteKey;
use aquilum_core::learning::state::{StateError, StudyStore};
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};
use masonry::kurbo::Point;
use super::super::{App, Popup};
use super::study::labels;
use crate::i18n::t;
use crate::ui::dialog::ConfirmDialog;

pub(crate) struct PendingTable {
    pub note: NoteKey,
    pub path: PathBuf,
    pub selected: EditorTableSnapshot,
}
pub(crate) struct PendingColumns {
    pub table: PendingTable,
    pub mapping: ColumnMapping,
}

fn group_choices(store: &StudyStore) -> Result<Vec<(String, String)>, StateError> {
    let mut choices = vec![("common".to_owned(), t("editor.table.studyDefault"))];
    for group in store.load()?.state.registry.collections {
        if group.id != "common" { choices.push((group.id, group.name)); }
    }
    choices.push((String::new(), t("editor.table.studyNew"))); choices.push(("__manage__".into(), t("editor.table.studyManage")));
    Ok(choices)
}

impl App {
    pub(in crate::app) fn open_study_add(&mut self, root: &mut RenderRoot) {
        let selected = (|| {
            let session = self.session.as_ref()?;
            let note = self.note.as_ref()?;
            if note.clean_cache_hash().is_none() || self.tabs.active().path.as_deref() != Some(note.path.as_path()) {
                return None;
            }
            let document = session.document(&self.workspace.core, &note.path)?;
            let snapshot = self.note_ids.as_ref()?.body.get(root)?
                .inner().study_table_snapshot().ok()?;
            Some(PendingTable {
                note: NoteKey { vault_id: session.workspace_id.to_string(), note_id: document.to_string() },
                path: note.path.clone(),
                selected: snapshot,
            })
        })();
        let Some(pending) = selected else {
            self.study_add_notice(root, &t("editor.table.studyStillChanged"), false);
            return;
        };
        if pending.selected.cells[0].len() < 2 {
            self.study_add_notice(root, &t("editor.table.studyNeedColumns"), false);
            return;
        }
        let entries = labels(&pending.selected, None);
        let layer = self.study_menu(root, &t("editor.table.studyQuestion"), &entries);
        self.popup = Some(Popup::StudyAddQuestion { layer, pending });
    }
    pub(in crate::app) fn on_study_add_question(
        &mut self, root: &mut RenderRoot, layer: WidgetId,
        pending: PendingTable, action: &ErasedAction,
    ) {
        let Some((index, pending)) = self.study_menu_item(root, layer, pending, action,
            |pending| Popup::StudyAddQuestion { layer, pending }) else { return };
        let choices = labels(&pending.selected, None);
        let Some((question, _)) = choices.get(index) else { return };
        let question = *question;
        let choices = labels(&pending.selected, Some(question));
        let layer = self.study_menu(root, &t("editor.table.studyAnswer"), &choices);
        self.popup = Some(Popup::StudyAddAnswer { layer, pending, question });
    }
    pub(in crate::app) fn on_study_add_answer(
        &mut self, root: &mut RenderRoot, layer: WidgetId,
        pending: PendingTable, question: usize, action: &ErasedAction,
    ) {
        let Some((index, pending)) = self.study_menu_item(root, layer, pending, action,
            |pending| Popup::StudyAddAnswer { layer, pending, question }) else { return };
        let options = labels(&pending.selected, Some(question));
        let Some((answer, _)) = options.get(index) else { return };
        let mapping = ColumnMapping { question, answers: vec![*answer], context: None };
        if mapping.validate(pending.selected.cells[0].len()).is_err() {
            self.study_add_notice(root, &t("editor.table.studyInvalidMapping"), false); return;
        }
        let store = StudyStore::open(&self.workspace.data_dir.join("language-study.sqlite"))
            .and_then(|store| group_choices(&store));
        let Ok(choices) = store else {
            self.study_add_notice(root, &t("editor.table.studySaveFailed"), false); return;
        };
        let entries = choices.iter().enumerate().map(|(i, (_, name))| (i, name.clone())).collect::<Vec<_>>();
        let layer = self.study_menu(root, &t("editor.table.studyChooseCollection"), &entries);
        self.popup = Some(Popup::StudyAddCollection {
            layer, pending: PendingColumns { table: pending, mapping }, choices,
        });
    }

    pub(in crate::app) fn on_study_add_collection(
        &mut self, root: &mut RenderRoot, layer: WidgetId,
        pending: PendingColumns, choices: Vec<(String, String)>, action: &ErasedAction,
    ) {
        let Some((index, (pending, choices))) = self.study_menu_item(root, layer, (pending, choices), action,
            |(pending, choices)| Popup::StudyAddCollection { layer, pending, choices }) else { return };
        let Some((id, name)) = choices.get(index) else { return };
        if id.is_empty() { self.open_study_new_collection(root, pending); return; } if id == "__manage__" { self.open_study_manage(root); return; }
        self.bind_live_table_with_notice(root, &pending, id, name);
    }

    pub(super) fn bind_live_table(
        &mut self, root: &RenderRoot, pending: &PendingColumns,
        group_id: &str, group_name: &str,
    ) -> Result<(), StateError> {
        let session = self.session.as_ref().ok_or(StateError::InvalidSelection)?;
        let note = self.note.as_ref().ok_or(StateError::InvalidSelection)?;
        if note.path != pending.table.path || note.clean_cache_hash().is_none()
            || self.tabs.active().path.as_deref() != Some(note.path.as_path())
        {
            return Err(StateError::InvalidSelection);
        }
        let document = session.document(&self.workspace.core, &note.path)
            .ok_or(StateError::InvalidSelection)?;
        if session.workspace_id.to_string() != pending.table.note.vault_id
            || document.to_string() != pending.table.note.note_id
        {
            return Err(StateError::InvalidSelection);
        }
        let complete = self.note_ids.as_ref().and_then(|ids| ids.body.get(root))
            .ok_or(StateError::InvalidSelection)?
            .inner().study_complete_note().map_err(|_| StateError::InvalidSelection)?;
        if complete.selected != pending.table.selected { return Err(StateError::InvalidSelection); }
        let mut store = StudyStore::open(&self.workspace.data_dir.join("language-study.sqlite"))?;
        let revision = store.load()?.revision;
        store.bind(
            revision, &pending.table.note, &complete.tables, complete.selected_index,
            pending.mapping.clone(), group_id, group_name, &uuid::Uuid::new_v4().to_string(),
        )?;
        Ok(())
    }

    pub(super) fn study_add_notice(&mut self, root: &mut RenderRoot, message: &str, success: bool) {
        let title = if success { "editor.table.studyAddedTitle" } else { "editor.table.studyTitle" };
        let (widget, dialog) = ConfirmDialog::new(
            &t(title), message, &t("editor.table.studyClose"), &t("editor.table.studyOk"),
        );
        root.add_layer(widget, Point::ORIGIN);
        root.focus_on(Some(dialog.layer));
        self.popup = Some(Popup::StudyPreview { dialog, copy: None });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_collection_is_available_without_stored_tables() {
        let dir = tempfile::tempdir().unwrap();
        let store = StudyStore::open(&dir.path().join("lang.sqlite")).unwrap();
        assert_eq!(group_choices(&store).unwrap()[0].0, "common");
    }
}
