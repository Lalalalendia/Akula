use masonry::core::{Ime, TextEvent};
use masonry::core::keyboard::{Key, Modifiers, NamedKey};

use super::support::{command_key, markdown_root, named, press, root, selected, text, typing};
use crate::test_dir::command;
use crate::ui::editor::Smart;
use crate::ui::editor::TextEditor;

#[test]
fn typing_and_enter_build_paragraphs() {
    let mut root = root("");
    typing(&mut root, "привет");
    named(&mut root, NamedKey::Enter);
    typing(&mut root, "мир");
    assert_eq!(text(&root), "привет\nмир");
}

#[test]
fn backspace_at_paragraph_start_joins_paragraphs() {
    let mut root = root("один\nдва");
    press(&mut root, Key::Named(NamedKey::End), command());
    for _ in 0..3 {
        named(&mut root, NamedKey::ArrowLeft);
    }
    named(&mut root, NamedKey::Backspace);
    assert_eq!(text(&root), "одиндва");
    named(&mut root, NamedKey::Delete);
    assert_eq!(text(&root), "одинва");
}

#[test]
fn edit_relayouts_only_its_paragraph() {
    let body: String = (0..2000).map(|i| format!("Абзац {i}: текст заметки, который переносится на несколько строк в колонке редактора.\n")).collect();
    let mut root = root(&body);
    press(&mut root, Key::Named(NamedKey::End), command());
    let started = std::time::Instant::now();
    typing(&mut root, "abcdefghij");
    let per_key = started.elapsed() / 10;
    assert!(text(&root).ends_with("abcdefghij"));
    assert!(per_key < std::time::Duration::from_millis(5), "нажатие в большом документе: {per_key:?}");
}

#[test]
fn undo_and_redo_group_typing_and_deleting() {
    let mut root = root("");
    typing(&mut root, "привет мир");
    named(&mut root, NamedKey::Backspace);
    named(&mut root, NamedKey::Backspace);
    assert_eq!(text(&root), "привет м");
    command_key(&mut root, "z");
    assert_eq!(text(&root), "привет мир");
    command_key(&mut root, "z");
    assert_eq!(text(&root), "");
    command_key(&mut root, "y");
    assert_eq!(text(&root), "привет мир");
    press(&mut root, Key::Character("z".into()), command() | Modifiers::SHIFT);
    assert_eq!(text(&root), "привет м");
    typing(&mut root, "!");
    command_key(&mut root, "y");
    assert_eq!(text(&root), "привет м!");
}

#[test]
fn redo_restores_the_selection_inside_wrapped_brackets() {
    let mut root = root("abc");
    command_key(&mut root, "a");
    typing(&mut root, "(");
    assert_eq!((text(&root).as_str(), selected(&root).as_str()), ("(abc)", "abc"));
    command_key(&mut root, "z");
    assert_eq!(text(&root), "abc");
    command_key(&mut root, "y");
    assert_eq!((text(&root).as_str(), selected(&root).as_str()), ("(abc)", "abc"));
}

#[test]
fn double_hyphen_and_space_become_dash() {
    let mut root = markdown_root("");
    let id = super::support::editor_id(&root);
    root.edit_widget(id, |mut w| TextEditor::set_smart(&mut w.downcast(), Smart { dashes: false, ..Default::default() }));
    typing(&mut root, "a-- b");
    assert_eq!(text(&root), "a-- b");
    root.edit_widget(id, |mut w| TextEditor::set_smart(&mut w.downcast(), Smart::default()));
    typing(&mut root, " c-- d--- e");
    assert_eq!(text(&root), "a-- b c— d--- e");
}

fn ime(root: &mut masonry::app::RenderRoot, event: Ime) {
    root.handle_text_event(TextEvent::Ime(event));
    let _ = root.redraw();
}

#[test]
fn ime_preedit_and_cancel_do_not_delete_selection_or_create_undo_history() {
    let original = "слева мир справа";
    let mut root = root(original);
    let start = "слева ".len();
    let end = start + "мир".len();
    let id = super::support::editor_id(&root);
    root.edit_widget(id, |mut w| TextEditor::select_byte_range(&mut w.downcast(), start, end));
    let _ = root.redraw();
    assert_eq!(selected(&root), "мир");

    ime(&mut root, Ime::Enabled);
    ime(&mut root, Ime::Preedit("に".into(), Some((3, 3))));
    ime(&mut root, Ime::Preedit("日本".into(), Some((6, 6))));
    assert_eq!(text(&root), original, "preedit must not change the document");
    assert_eq!(selected(&root), "мир", "preedit must retain the selection");

    ime(&mut root, Ime::Disabled);
    assert_eq!(text(&root), original, "cancelling must preserve the document");
    assert_eq!(selected(&root), "мир", "cancelling must preserve the selection");
    command_key(&mut root, "z");
    assert_eq!(text(&root), original, "cancel must not add an undo step");
}

#[test]
fn ime_commit_replaces_selection_once_and_one_undo_restores_it() {
    let original = "слева мир справа";
    let mut root = root(original);
    let start = "слева ".len();
    let end = start + "мир".len();
    let id = super::support::editor_id(&root);
    root.edit_widget(id, |mut w| TextEditor::select_byte_range(&mut w.downcast(), start, end));
    let _ = root.redraw();

    ime(&mut root, Ime::Enabled);
    ime(&mut root, Ime::Preedit("に".into(), Some((3, 3))));
    ime(&mut root, Ime::Preedit("日本".into(), Some((6, 6))));
    ime(&mut root, Ime::Preedit(String::new(), None));
    assert_eq!(text(&root), original, "empty preedit is transient too");
    ime(&mut root, Ime::Commit("日本語".into()));
    assert_eq!(text(&root), "слева 日本語 справа");

    command_key(&mut root, "z");
    assert_eq!(text(&root), original, "one undo restores the full selection");
    command_key(&mut root, "z");
    assert_eq!(text(&root), original, "preedit must not add a second undo step");
    command_key(&mut root, "y");
    assert_eq!(text(&root), "слева 日本語 справа");
}

#[test]
fn ime_empty_commit_preserves_multiline_selection_and_undo() {
    let original = "начало\nсередина\n日本語";
    let mut root = root(original);
    let start = original.find("середина").unwrap();
    let end = original.len();
    let selected_original = &original[start..end];
    let id = super::support::editor_id(&root);
    root.edit_widget(id, |mut w| TextEditor::select_byte_range(&mut w.downcast(), start, end));
    let _ = root.redraw();
    assert_eq!(selected(&root), selected_original);

    // OS IMEs may finish a cancelled or emptied composition with Commit("").
    // Unlike a real nonempty Commit, it must not delete any selected bytes.
    ime(&mut root, Ime::Enabled);
    ime(&mut root, Ime::Preedit("に".into(), Some((3, 3))));
    ime(&mut root, Ime::Preedit(String::new(), None));
    ime(&mut root, Ime::Commit(String::new()));
    assert_eq!(text(&root), original, "empty commit must not alter document bytes");
    assert_eq!(selected(&root), selected_original, "empty commit must retain selection");
    command_key(&mut root, "z");
    assert_eq!(text(&root), original, "no empty commit undo entry is allowed");

    // A second empty commit while a nonempty preedit is active is also inert.
    ime(&mut root, Ime::Preedit("日本".into(), Some((6, 6))));
    ime(&mut root, Ime::Commit(String::new()));
    assert_eq!(text(&root), original);
    assert_eq!(selected(&root), selected_original);
}

#[test]
fn ime_multiline_selection_preedit_cancel_preserves_every_byte() {
    let original = "лево 🍀\n日本語 содержимое\nпосле ✅ хвост";
    let start = original.find("🍀").unwrap();
    let end = original.find(" хвост").unwrap();
    let selected_original = &original[start..end];
    let mut root = root(original);
    let id = super::support::editor_id(&root);
    root.edit_widget(id, |mut widget| {
        TextEditor::select_byte_range(&mut widget.downcast(), start, end);
    });
    let _ = root.redraw();

    ime(&mut root, Ime::Enabled);
    for (preview, cursor) in [("に", 3), ("日本", 6), ("日本語", 9)] {
        ime(&mut root, Ime::Preedit(preview.into(), Some((cursor, cursor))));
        assert_eq!(text(&root), original, "provisional text must not change stored Unicode bytes");
        assert_eq!(selected(&root), selected_original, "preedit must retain all selected paragraphs");
    }
    ime(&mut root, Ime::Disabled);
    assert_eq!(text(&root), original, "cancelling a multiline composition must leave no edits");
    assert_eq!(selected(&root), selected_original, "cancel must not collapse selection");
    command_key(&mut root, "z");
    assert_eq!(text(&root), original, "cancel may not create an Undo entry");
}

#[test]
fn ime_multiline_selection_nonempty_commit_is_single_undoable_replacement() {
    let original = "слева 🍀\n日本語 содержимое\nпосле ✅ хвост";
    let start = original.find("🍀").unwrap();
    let end = original.find(" хвост").unwrap();
    let expected = format!("{}選択済{}", &original[..start], &original[end..]);
    let mut root = root(original);
    let id = super::support::editor_id(&root);
    root.edit_widget(id, |mut widget| {
        TextEditor::select_byte_range(&mut widget.downcast(), start, end);
    });
    let _ = root.redraw();

    ime(&mut root, Ime::Enabled);
    ime(&mut root, Ime::Preedit("に".into(), Some((3, 3))));
    ime(&mut root, Ime::Preedit("日本".into(), Some((6, 6))));
    assert_eq!(text(&root), original);
    ime(&mut root, Ime::Commit("選択済".into()));
    assert_eq!(text(&root), expected, "commit replaces all selected lines exactly once");

    command_key(&mut root, "z");
    assert_eq!(text(&root), original, "one Undo restores Unicode and embedded newline bytes");
    command_key(&mut root, "z");
    assert_eq!(text(&root), original, "Preedit did not add another Undo entry");
    command_key(&mut root, "y");
    assert_eq!(text(&root), expected, "Redo restores exactly one committed replacement");
}

#[test]
fn ime_window_blur_does_not_delete_selected_unicode_or_create_history() {
    let original = "доверие 🍀\n日本語 и кириллица";
    let start = original.find("🍀").unwrap();
    let end = original.find(" и кириллица").unwrap();
    let mut root = root(original);
    let id = super::support::editor_id(&root);
    root.edit_widget(id, |mut widget| {
        TextEditor::select_byte_range(&mut widget.downcast(), start, end);
    });
    let _ = root.redraw();

    ime(&mut root, Ime::Enabled);
    ime(&mut root, Ime::Preedit("に".into(), Some((3, 3))));
    root.handle_text_event(TextEvent::WindowFocusChange(false));
    let _ = root.redraw();
    assert_eq!(text(&root), original);
    assert_eq!(selected(&root), original[start..end]);
    root.focus_on(None);
    let _ = root.redraw();
    root.focus_on(Some(id));
    let _ = root.redraw();
    assert_eq!(text(&root), original);
    command_key(&mut root, "z");
    assert_eq!(text(&root), original, "losing focus during preedit must not add Undo history");
}
