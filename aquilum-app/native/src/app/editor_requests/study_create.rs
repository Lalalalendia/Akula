//! Create a named study collection from the current table without changing
//! Markdown, granting file access or copying source vocabulary to metadata.

use aquilum_core::learning::state::{StateError, StudyStore};
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};

use super::super::{App, Popup};
use super::text_input_dialog::open_input_dialog;
use super::study_add::PendingColumns;
use crate::i18n::t;
use crate::ui::dialog::{DialogAnswer, DialogIds};
use crate::ui::editor::{TextAction, TextEditor};
use crate::ui::handle::Handle;
use crate::ui::search::Placeholder;

fn clean_name(raw: &str) -> Option<String> {
    if raw.len() > 256 { return None; }
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(|c| {
        c.is_control() || matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    }) {
        return None;
    }
    Some(name.to_owned())
}

fn name_available(store: &StudyStore, proposed: &str) -> Result<bool, StateError> {
    if proposed.to_lowercase() == t("editor.table.studyDefault").to_lowercase() { return Ok(false); }
    Ok(store.load()?.state.registry.collections.iter().all(|group|
        !group.name.eq_ignore_ascii_case(proposed) && group.name.to_lowercase() != proposed.to_lowercase()))
}

impl App {
    pub(in crate::app) fn open_study_new_collection(
        &mut self, root: &mut RenderRoot, pending: PendingColumns,
    ) {
        let title = t("editor.table.studyCreateTitle");
        let (dialog, field, placeholder) =
            open_input_dialog(root, &title, "", "editor.table.studyNameHint", "editor.table.studyClose", "editor.table.studyCreate");
        self.popup = Some(Popup::StudyNewCollection { dialog, field, placeholder, pending, value: String::new() });
    }

    pub(in crate::app) fn on_study_new_collection(
        &mut self, root: &mut RenderRoot, id: WidgetId, dialog: DialogIds,
        field: Handle<TextEditor>, placeholder: Handle<Placeholder>,
        pending: PendingColumns, mut value: String, action: &ErasedAction,
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
            self.popup = Some(Popup::StudyNewCollection { dialog, field, placeholder, pending, value });
            return;
        }
        let Some(answer) = dialog.answer(id, action) else {
            self.popup = Some(Popup::StudyNewCollection { dialog, field, placeholder, pending, value });
            return;
        };
        root.remove_layer(dialog.layer);
        if answer != DialogAnswer::Confirm { return; }
        let Some(name) = clean_name(&value) else {
            self.study_add_notice(root, &t("editor.table.studyInvalidName"), false);
            return;
        };
        let duplicate = StudyStore::open(&self.workspace.data_dir.join("language-study.sqlite"))
            .and_then(|store| name_available(&store, &name));
        match duplicate {
            Ok(false) => {
                self.study_add_notice(root, &t("editor.table.studyDuplicateName"), false);
                return;
            }
            Err(_) => {
                self.study_add_notice(root, &t("editor.table.studySaveFailed"), false);
                return;
            }
            Ok(true) => {}
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.bind_live_table_with_notice(root, &pending, &id, &name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_unicode_and_trims_space() {
        assert_eq!(clean_name("  Фэнтези / жанры  "), Some("Фэнтези / жанры".into()));
        assert_eq!(clean_name("Verb conjugations"), Some("Verb conjugations".into()));
    }

    #[test]
    fn rejects_ambiguous_invisible_controls_and_unbounded_text() {
        for s in ["", "  ", "Example\nOther", "word\u{202e}unsafe", "word\u{200e}unsafe"] {
            assert_eq!(clean_name(s), None);
        }
        assert_eq!(clean_name(&"a".repeat(257)), None);
        assert_eq!(clean_name(&"я".repeat(65)), None);
    }

    #[test]
    fn rejects_existing_collection_names_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("study.sqlite");
        let mut store = StudyStore::open(&file).unwrap();
        let key = aquilum_core::learning::table_anchors::NoteKey {
            vault_id: "vault-1".into(), note_id: "note-1".into(),
        };
        let rows = [vec![vec!["Q".into(), "A".into()], vec!["Run".into(), "Бежать".into()]]];
        let columns = aquilum_core::learning::ColumnMapping { question: 0, answers: vec![1], context: None };
        store.bind(0, &key, &rows, 0, columns, "verbs", "Глаголы", "table-1").unwrap();
        assert!(!name_available(&store, "гЛаГоЛы").unwrap());
        assert!(name_available(&store, "Существительные").unwrap());
    }
}
