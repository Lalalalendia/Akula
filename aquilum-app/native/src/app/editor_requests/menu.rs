use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};
use masonry::kurbo::Point;

use super::super::{App, Popup};
use crate::frontmatter;
use crate::i18n::t;
use crate::ui::editor::{TableCommand, TextEditor};
use crate::ui::icons;
use crate::ui::menu::{Entry, Menu, MenuChoice};

#[derive(Clone, Debug, PartialEq)]
pub enum EditorCommand {
    Copy,
    CopyCode(String),
    Paste,
    InsertBook,
    InsertTable,
    InsertFrontmatter,
}

impl App {
    pub(in crate::app) fn open_table_menu(&mut self, root: &mut RenderRoot, at: Point, commands: Vec<(TableCommand, bool)>) {
        let labels: Vec<String> = commands.iter().map(|(c, _)| t(c.label_key())).collect();
        let entries: Vec<Entry<'_>> = commands.iter().zip(&labels).map(|((c, enabled), label)| Entry { label, icon: Some(c.icon()), disabled: !enabled }).collect();
        let menu = Menu::entries(&t("editor.table.menuAria"), &entries);
        let commands: Vec<TableCommand> = commands.into_iter().map(|(c, _)| c).collect();
        let layer = menu.id();
        root.add_layer(menu, Menu::context_origin(at, entries.len(), self.window));
        root.focus_on(Some(layer));
        self.popup = Some(Popup::TableMenu { layer, commands });
    }

    pub(in crate::app) fn on_table_menu(&mut self, root: &mut RenderRoot, layer: WidgetId, commands: Vec<TableCommand>, action: &ErasedAction) {
        let Some(choice) = action.downcast_ref::<MenuChoice>() else {
            self.popup = Some(Popup::TableMenu { layer, commands });
            return;
        };
        let MenuChoice::Item(item) = choice else { return };
        root.remove_layer(layer);
        let Some(&command) = commands.get(*item) else { return };
        if let Some(ids) = &self.note_ids {
            let body = ids.body.id();
            root.focus_on(Some(body));
            ids.body.edit(root, |mut editor| TextEditor::table_command(&mut editor, command));
        }
    }

    pub(in crate::app) fn open_editor_context_menu(&mut self, root: &mut RenderRoot, at: Point, has_selection: bool, code: Option<String>) {
        let has_frontmatter = frontmatter::end_of(&self.text).is_some();
        let mut commands = vec![EditorCommand::Copy];
        if let Some(source) = code {
            commands.push(EditorCommand::CopyCode(source));
        }
        commands.extend([
            EditorCommand::Paste,
            EditorCommand::InsertBook,
            EditorCommand::InsertTable,
            EditorCommand::InsertFrontmatter,
        ]);
        let labels: Vec<String> = commands.iter().map(|command| t(match command {
            EditorCommand::Copy => "common.copy",
            EditorCommand::CopyCode(_) => "editor.copyCode",
            EditorCommand::Paste => "common.paste",
            EditorCommand::InsertBook => "editor.insertBook",
            EditorCommand::InsertTable => "editor.insertTable",
            EditorCommand::InsertFrontmatter => "editor.insertFrontmatter",
        })).collect();
        let entries: Vec<Entry<'_>> = commands.iter().zip(&labels).map(|(command, label)| {
            let (icon, disabled) = match command {
                EditorCommand::Copy => (icons::COPY, !has_selection),
                EditorCommand::CopyCode(_) => (icons::COPY, false),
                EditorCommand::Paste => (icons::CLIPBOARD, false),
                EditorCommand::InsertBook => (icons::BOOK_OPEN, false),
                EditorCommand::InsertTable => (icons::TABLE, false),
                EditorCommand::InsertFrontmatter => (icons::TAG, has_frontmatter),
            };
            Entry { label, icon: Some(icon), disabled }
        }).collect();
        let menu = Menu::entries(&t("editor.actions"), &entries);
        let layer = menu.id();
        root.add_layer(menu, Menu::context_origin(at, entries.len(), self.window));
        root.focus_on(Some(layer));
        self.popup = Some(Popup::EditorMenu { layer, commands });
    }

    pub(in crate::app) fn on_editor_menu(&mut self, root: &mut RenderRoot, layer: WidgetId, commands: Vec<EditorCommand>, action: &ErasedAction) {
        let Some(choice) = action.downcast_ref::<MenuChoice>() else {
            self.popup = Some(Popup::EditorMenu { layer, commands });
            return;
        };
        let MenuChoice::Item(item) = choice else { return };
        root.remove_layer(layer);
        let Some(&command) = commands.get(*item) else { return };
        let Some(ids) = &self.note_ids else { return };
        let body = ids.body;
        root.focus_on(Some(body.id()));
        match command {
            EditorCommand::Copy => {
                let selected = body.get(root).and_then(|e| e.inner().selected_text());
                if let Some(text) = selected
                    && let Ok(mut cb) = arboard::Clipboard::new()
                {
                    let _ = cb.set_text(text);
                }
            }
            EditorCommand::CopyCode(text) => {
                // Clipboard-only: do not signal a document edit or autosave.
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    let _ = cb.set_text(text.clone());
                }
            }
            EditorCommand::Paste => {
                if let Ok(mut cb) = arboard::Clipboard::new()
                    && let Ok(text) = cb.get_text()
                {
                    body.edit(root, |mut e| TextEditor::insert_text(&mut e, &text));
                }
            }
            EditorCommand::InsertBook => {
                body.edit(root, |mut e| TextEditor::insert_text(&mut e, "> [!book] [[Название книги]]\n"));
            }
            EditorCommand::InsertTable => {
                body.edit(root, |mut e| TextEditor::insert_table(&mut e, 2, 2));
            }
            EditorCommand::InsertFrontmatter => {
                if frontmatter::end_of(&self.text).is_none() {
                    let mut text = self.text.clone();
                    text.insert(0, frontmatter::template(&[(frontmatter::TAGS, "[]")]));
                    self.commit_text(root, text);
                }
            }
        }
    }
}
