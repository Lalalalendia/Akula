use super::*;

#[test]
fn save_error_slot_only_rebuilds_when_a_banner_changes() {
    use crate::ui::widgets::Slot;

    let mut h = Harness::new();
    let first = h.write_note("save-error-a.md", "first note");
    let second = h.write_note("save-error-b.md", "second note");
    assert!(h.app.open(&mut h.root, first));
    assert!(h.app.save_error_ids.is_none());

    // A clean page already has an empty error Slot: repeated refresh must
    // not cause an unrelated widget subtree replacement.
    let _ = Slot::take_replace_timings();
    h.app.refresh_save_error(&mut h.root);
    assert!(Slot::take_replace_timings().is_none());
    assert!(h.app.save_error_ids.is_none());

    // An actual failure still installs the banner and its retry action.
    h.app.note.as_mut().unwrap().save_error = Some("disk unavailable".into());
    h.app.refresh_save_error(&mut h.root);
    assert!(Slot::take_replace_timings().is_some());
    assert!(h.app.save_error_ids.is_some());

    // Clearing the failure removes the visible banner exactly once.
    h.app.note.as_mut().unwrap().save_error = None;
    h.app.refresh_save_error(&mut h.root);
    assert!(Slot::take_replace_timings().is_some());
    assert!(h.app.save_error_ids.is_none());

    h.app.refresh_save_error(&mut h.root);
    assert!(Slot::take_replace_timings().is_none());

    // The next note begins with a fresh Slot and must never retain the
    // previous page's retry widget ID.
    h.app.note.as_mut().unwrap().save_error = Some("previous-page error".into());
    h.app.refresh_save_error(&mut h.root);
    assert!(h.app.save_error_ids.is_some());
    assert!(h.app.open(&mut h.root, second));
    assert!(h.app.save_error_ids.is_none());
    assert_eq!(h.body_text(), "second note");
    let _ = h.root.redraw();
}


#[test]
fn conflicting_edit_is_kept_as_a_copy() {
    let mut h = Harness::new();
    let path = h.vault.path().join("a.md");
    h.app.open(&mut h.root, path.clone());
    std::fs::write(&path, "снаружи").unwrap();
    h.app.text = crop::Rope::from("моя правка");
    h.app.note.as_mut().unwrap().edit(std::time::Duration::ZERO);
    h.save_now();
    let copies: Vec<_> = std::fs::read_dir(h.vault.path()).unwrap().flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().contains("конфликт")).collect();
    assert_eq!(copies.len(), 1, "копия-конфликт не записана");
    assert!(std::fs::read_to_string(&copies[0]).unwrap().contains("моя правка"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "снаружи");
    assert!(h.app.text.to_string().contains("снаружи"), "{}", h.app.text.to_string());
}

#[test]
fn inserted_text_reaches_the_disk() {
    let mut h = Harness::new();
    let path = h.vault.path().join("a.md");
    h.app.open(&mut h.root, path.clone());
    h.app.commit_text(&mut h.root, crop::Rope::from("---\ntags: []\n---\nтекст"));
    h.save_now();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "---\ntags: []\n---\nтекст");
    assert_eq!(h.app.metadata_state(), Some(true), "вставленные метаданные раскрыты");
}

#[test]
fn unsaved_text_is_written_on_shutdown() {
    let mut h = Harness::new();
    let path = h.vault.path().join("a.md");
    h.app.open(&mut h.root, path.clone());
    h.app.commit_text(&mut h.root, crop::Rope::from("несохранённая правка"));
    h.app.shutdown();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "несохранённая правка");
}

#[test]
fn typed_edits_reach_the_file_behind_hidden_frontmatter() {
    let mut h = Harness::new();
    let path = h.write_note("правки.md", "---\ntags: [a]\n---\nпривет мир");
    h.app.open(&mut h.root, path.clone());
    assert!(h.app.hidden > 0, "метаданные должны быть скрыты");
    let body = h.app.note_ids.as_ref().unwrap().body;
    let end = h.body_text().len();
    h.root.focus_on(Some(body.id()));
    body.edit(&mut h.root, |mut b| crate::ui::editor::TextEditor::select_byte_range(&mut b, end, end));
    h.type_text("!ё");
    h.key(Key::Named(NamedKey::Backspace), Modifiers::empty());
    body.edit(&mut h.root, |mut b| crate::ui::editor::TextEditor::select_byte_range(&mut b, 0, 0));
    h.key(Key::Character("Ж".into()), Modifiers::empty());
    assert_eq!(h.app.text.to_string(), "---\ntags: [a]\n---\nЖпривет мир!");
    h.save_now();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "---\ntags: [a]\n---\nЖпривет мир!");
}

#[test]
fn full_app_undo_after_30k_line_paste_restores_gui_and_disk_bytes() {
    // Exercise the complete RenderRoot -> App::body_edited -> Note::save path,
    // rather than only TextEditor state without a real persisted note.
    let mut h = Harness::new();
    let original: String =
        (0..30_000).map(|n| format!("строка {n} — проверка окна Markdown редактора\n")).collect();
    let path = h.write_note("Long undo.md", &original);
    assert!(h.app.open(&mut h.root, path.clone()));
    h.focus_editor();
    h.key(Key::Character("a".into()), crate::test_dir::command());

    let replacement = "AKULA_INTEGRATION_REPLACE_123456789";
    h.root.handle_text_event(TextEvent::ClipboardPaste(replacement.into()));
    h.deliver();
    assert_eq!(h.body_text(), replacement);
    assert_eq!(h.app.text.to_string(), replacement);
    h.save_now();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), replacement);

    h.key(Key::Character("z".into()), crate::test_dir::command());
    assert_eq!(h.body_text(), original, "editor undo lost bytes");
    assert_eq!(h.app.text.to_string(), original, "App missed the undo edit action");
    h.save_now();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original, "File Gate did not publish the full undo");
}

#[test]
fn large_note_typing_keeps_widget_and_saved_text_identical_with_hidden_unicode_frontmatter() {
    let mut h = Harness::new();
    let header = "---\ntags: [тест]\n---\n";
    let body = "Кириллица 😀 и длинная строка редактирования.\n".repeat(3_200);
    let original = format!("{header}{body}");
    let path = h.write_note("Длинная заметка.md", &original);
    h.app.open(&mut h.root, path.clone());
    assert!(h.app.hidden > 0, "frontmatter should be hidden");

    let editor = h.app.note_ids.as_ref().unwrap().body;
    // A direct selection outside the virtualized viewport does not itself
    // scroll/lay out that region. Insert via the widget API, which applies the
    // edit and emits the same Edited action used by the interactive editor.
    editor.edit(&mut h.root, |mut widget| {
        crate::ui::editor::TextEditor::select_byte_range(&mut widget, body.len(), body.len());
        crate::ui::editor::TextEditor::insert_text(&mut widget, "Ж");
    });
    h.deliver();

    let expected = format!("{original}Ж");
    assert_eq!(h.body_text(), format!("{body}Ж"));
    assert_eq!(h.app.text.to_string(), expected);
    h.save_now();
    assert_eq!(std::fs::read_to_string(path).unwrap(), expected);
}
