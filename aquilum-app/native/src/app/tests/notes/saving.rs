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
fn oversized_recovery_warning_is_visible_before_save_and_clears_after_file_gate_ack() {
    let mut h = Harness::new();
    let path = h.write_note("oversized-unsaved-note.md", "original disk contents");
    assert!(h.app.open(&mut h.root, path.clone()));
    assert!(h.app.save_error_ids.is_none());

    let line = format!("{}\n", "x".repeat(191));
    let draft = line.repeat(90_000);
    assert!(draft.len() > 16 * 1024 * 1024);
    h.app.commit_text(&mut h.root, crop::Rope::from(draft.clone()));
    assert!(h.app.note.as_ref().unwrap().save_error.as_ref().is_some_and(|error| error.contains("16 MiB")));
    assert!(h.app.save_error_ids.is_some(), "over-limit warning must be visible without polling or fsync");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original disk contents");

    h.save_now();
    crate::disk::wait();
    assert_eq!(std::fs::read(&path).unwrap(), draft.into_bytes());
    assert!(h.app.note.as_ref().unwrap().save_error.is_none());
    assert!(h.app.save_error_ids.is_none(), "verified File Gate ACK must clear transient oversize warning");
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
fn two_dirty_tabs_survive_conflicting_first_write_and_second_crash() {
    use crate::ui::dialog::DialogAnswer;

    let mut h = Harness::new();
    let first = h.write_note("two-dirty-first.md", "first baseline");
    let second = h.write_note("two-dirty-second.md", "second baseline");
    assert!(h.app.open(&mut h.root, first.clone()));
    let workspace = h.app.session.as_ref().unwrap().workspace_id;
    let first_draft = "first unsaved Unicode 😀";
    let second_draft = "second unsaved Unicode 漢字";
    h.app.commit_text(&mut h.root, crop::Rope::from(first_draft));
    crate::disk::wait();
    std::fs::write(&first, "external writer changed first").unwrap();

    // Switching away queues the first write. File Gate must not treat the
    // externally diverged file as a successful ACK or retire its shadow.
    assert!(h.app.open_in(&mut h.root, second.clone(), true));
    crate::disk::wait();
    crate::disk::wait();
    assert_eq!(std::fs::read_to_string(&first).unwrap(), "external writer changed first");
    let first_copy = crate::note::bridge::inspect_for_recovery(
        &h.app.workspace.data_dir, h.vault.path(), workspace, &first,
    ).unwrap().unwrap();
    assert_eq!(first_copy.body.to_string(), first_draft);
    assert!(first_copy.source_changed);

    h.app.commit_text(&mut h.root, crop::Rope::from(second_draft));
    crate::disk::wait();
    drop(h.app.note.take()); // hard exit before second File Gate debounce
    let second_copy = crate::note::bridge::inspect_for_recovery(
        &h.app.workspace.data_dir, h.vault.path(), workspace, &second,
    ).unwrap().unwrap();
    assert_eq!(second_copy.body.to_string(), second_draft);
    assert!(!second_copy.source_changed);
    assert_eq!(std::fs::read_to_string(&second).unwrap(), "second baseline");

    h.app.show_active(&mut h.root);
    crate::disk::wait();
    h.app.poll_note_recovery(&mut h.root);
    let layer = match &h.app.popup {
        Some(Popup::Recovery(state)) => state.dialog.layer,
        _ => panic!("second unsaved tab must offer explicit recovery"),
    };
    h.send(layer, DialogAnswer::Cancel);
    assert!(h.app.popup.is_none());

    assert!(h.app.open(&mut h.root, first.clone()));
    crate::disk::wait();
    h.app.poll_note_recovery(&mut h.root);
    assert!(matches!(h.app.popup, Some(Popup::Recovery(_))),
        "conflicted first tab must independently request review");
    assert_eq!(std::fs::read_to_string(&first).unwrap(), "external writer changed first");
    assert_eq!(std::fs::read_to_string(&second).unwrap(), "second baseline");
    let second_after = crate::note::bridge::inspect_for_recovery(
        &h.app.workspace.data_dir, h.vault.path(), workspace, &second,
    ).unwrap().unwrap();
    assert_eq!(second_after.body.to_string(), second_draft);
}

#[test]
fn switching_tabs_retires_only_acknowledged_shadow_and_preserves_crashed_tab() {
    let mut h = Harness::new();
    let first = h.write_note("recover-switch-first.md", "original first");
    let second = h.write_note("recover-switch-second.md", "original second");
    assert!(h.app.open(&mut h.root, first.clone()));
    let workspace = h.app.session.as_ref().unwrap().workspace_id;
    h.app.commit_text(&mut h.root, crop::Rope::from("saved after switching tabs"));
    crate::disk::wait(); // publish the first immutable shadow
    assert!(crate::note::bridge::inspect_for_recovery(
        &h.app.workspace.data_dir, h.vault.path(), workspace, &first,
    ).unwrap().is_some());

    assert!(h.app.open_in(&mut h.root, second.clone(), true));
    // Note::leave queues File Gate and detaches the Note. The worker must
    // still observe its saved ACK and retire the matching plaintext shadow.
    crate::disk::wait();
    crate::disk::wait(); // cleanup is queued after the detached ACK collector
    assert_eq!(std::fs::read_to_string(&first).unwrap(), "saved after switching tabs");
    assert!(crate::note::bridge::inspect_for_recovery(
        &h.app.workspace.data_dir, h.vault.path(), workspace, &first,
    ).unwrap().is_none(), "tab switch left an already-saved plaintext backup");

    h.app.commit_text(&mut h.root, crop::Rope::from("unsaved second draft 🐱"));
    crate::disk::wait();
    drop(h.app.note.take()); // simulate a hard stop BEFORE this tab's File Gate save
    assert_eq!(std::fs::read_to_string(&second).unwrap(), "original second");
    let candidate = crate::note::bridge::inspect_for_recovery(
        &h.app.workspace.data_dir, h.vault.path(), workspace, &second,
    ).unwrap().unwrap();
    assert_eq!(candidate.body, "unsaved second draft 🐱");
    h.app.show_active(&mut h.root);
    crate::disk::wait();
    h.app.poll_note_recovery(&mut h.root);
    assert!(matches!(h.app.popup, Some(Popup::Recovery(_))),
        "the second tab's unsaved text must still be reviewable");
    assert_eq!(std::fs::read_to_string(&first).unwrap(), "saved after switching tabs");
    assert_eq!(std::fs::read_to_string(&second).unwrap(), "original second");
}

#[test]
fn dirty_edit_offers_explicit_review_after_window_note_is_lost() {
    let mut h = Harness::new();
    let path = h.write_note("recover.md", "original");
    assert!(h.app.open(&mut h.root, path.clone()));
    let workspace = h.app.session.as_ref().unwrap().workspace_id;
    h.app.commit_text(&mut h.root, crop::Rope::from("несохранённый текст 😀"));
    crate::disk::wait();
    let candidate = crate::note::bridge::inspect_for_recovery(
        &h.app.workspace.data_dir, h.vault.path(), workspace, &path,
    ).unwrap().unwrap();
    assert_eq!(candidate.body.to_string(), "несохранённый текст 😀");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
    drop(h.app.note.take()); // no File Gate flush
    h.app.show_active(&mut h.root);
    crate::disk::wait();
    h.app.poll_note_recovery(&mut h.root);
    assert!(matches!(h.app.popup, Some(Popup::Recovery(_))));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "original");
}

#[test]
fn closing_recovery_modal_never_sticks_or_discards_the_shadow() {
    use crate::ui::dialog::DialogAnswer;
    let mut h = Harness::new();
    let path = h.write_note("recover-close.md", "persisted");
    assert!(h.app.open(&mut h.root, path.clone()));
    let workspace = h.app.session.as_ref().unwrap().workspace_id;
    h.app.commit_text(&mut h.root, crop::Rope::from("unsaved private draft"));
    crate::disk::wait();
    drop(h.app.note.take());
    h.app.show_active(&mut h.root);
    crate::disk::wait();
    h.app.poll_note_recovery(&mut h.root);
    let layer = match &h.app.popup {
        Some(Popup::Recovery(state)) => state.dialog.layer,
        _ => panic!("recovery confirmation expected"),
    };
    h.send(layer, DialogAnswer::Closed);
    assert!(h.app.popup.is_none(), "closed modal must unlock other actions");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "persisted");
    let record = crate::note::bridge::inspect_for_recovery(
        &h.app.workspace.data_dir, h.vault.path(), workspace, &path,
    ).unwrap().unwrap();
    assert_eq!(record.body.to_string(), "unsaved private draft");
}

#[test]
fn recovery_confirm_writes_through_file_gate_and_clears_only_after_ack() {
    use crate::ui::dialog::DialogAnswer;
    let mut h = Harness::new();
    let path = h.write_note("recover-confirm.md", "confirmed disk");
    assert!(h.app.open(&mut h.root, path.clone()));
    let workspace = h.app.session.as_ref().unwrap().workspace_id;
    let draft = "unsaved кириллица 🐱";
    h.app.commit_text(&mut h.root, crop::Rope::from(draft));
    crate::disk::wait();
    drop(h.app.note.take());
    h.app.show_active(&mut h.root);
    crate::disk::wait();
    h.app.poll_note_recovery(&mut h.root);
    let layer = match &h.app.popup {
        Some(Popup::Recovery(state)) => state.dialog.layer,
        _ => panic!("expected a recovery prompt"),
    };
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "confirmed disk");
    h.send(layer, DialogAnswer::Confirm);
    assert!(h.app.popup.is_none());
    assert_eq!(h.app.text.to_string(), draft);
    h.save_now();
    crate::disk::wait();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), draft);
    assert!(crate::note::bridge::inspect_for_recovery(
        &h.app.workspace.data_dir, h.vault.path(), workspace, &path,
    ).unwrap().is_none());
}

#[test]
fn recovery_of_external_conflict_leaves_original_and_backup_unchanged() {
    use crate::ui::dialog::DialogAnswer;
    let mut h = Harness::new();
    let path = h.write_note("recover-conflict.md", "old disk");
    assert!(h.app.open(&mut h.root, path.clone()));
    let workspace = h.app.session.as_ref().unwrap().workspace_id;
    h.app.commit_text(&mut h.root, crop::Rope::from("my unsaved 🐱"));
    crate::disk::wait();
    drop(h.app.note.take());
    std::fs::write(&path, "external edit").unwrap();
    h.app.show_active(&mut h.root);
    crate::disk::wait();
    h.app.poll_note_recovery(&mut h.root);
    let layer = match &h.app.popup {
        Some(Popup::Recovery(state)) => state.dialog.layer,
        _ => panic!("external conflict should offer manual review"),
    };
    h.send(layer, DialogAnswer::Cancel);
    assert!(h.app.popup.is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "external edit");
    let record = crate::note::bridge::inspect_for_recovery(
        &h.app.workspace.data_dir, h.vault.path(), workspace, &path,
    ).unwrap().unwrap();
    assert_eq!(record.body.to_string(), "my unsaved 🐱");
    assert!(record.source_changed);
}

#[test]
fn recovery_confirmation_rechecks_external_edit_after_dialog_is_shown() {
    use crate::ui::dialog::DialogAnswer;
    let mut h = Harness::new();
    let path = h.write_note("confirm-race.md", "last confirmed");
    assert!(h.app.open(&mut h.root, path.clone()));
    let workspace = h.app.session.as_ref().unwrap().workspace_id;
    let draft = "unpublished private draft 🐱";
    h.app.commit_text(&mut h.root, crop::Rope::from(draft));
    crate::disk::wait();
    drop(h.app.note.take());
    h.app.show_active(&mut h.root);
    crate::disk::wait();
    h.app.poll_note_recovery(&mut h.root);
    let layer = match &h.app.popup {
        Some(Popup::Recovery(state)) => state.dialog.layer,
        _ => panic!("recovery prompt was not shown"),
    };
    // The source changes AFTER the initial recovery dialog is created.
    std::fs::write(&path, "external update during confirmation").unwrap();
    h.send(layer, DialogAnswer::Confirm);
    assert!(h.app.popup.is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "external update during confirmation");
    assert_ne!(h.app.text.to_string(), draft, "stale candidate must not enter the editor");
    let record = crate::note::bridge::inspect_for_recovery(
        &h.app.workspace.data_dir, h.vault.path(), workspace, &path,
    ).unwrap().unwrap();
    assert_eq!(record.body.to_string(), draft);
    assert!(record.source_changed);
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
