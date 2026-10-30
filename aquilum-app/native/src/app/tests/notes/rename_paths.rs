//! Native user-entrypoint regressions after the safe rename transaction (#913).
//! Only disposable Harness vaults are touched; no published installer or user data.
use super::*;
use crate::ui::editor::TextEditor;

#[test]
fn editing_note_title_then_focusing_body_commits_real_rename_without_enter() {
    let mut h = Harness::new();
    let original = h.vault.path().join("a.md");
    let renamed = h.vault.path().join("B.md");
    h.app.open(&mut h.root, original.clone());
    let ids = h.app.note_ids.as_ref().unwrap();
    let (title, body) = (ids.title, ids.body);
    h.root.focus_on(Some(title.id()));
    title.edit(&mut h.root, |mut field| {
        let length = field.widget.len();
        TextEditor::select_byte_range(&mut field, 0, length);
    });
    let _ = h.root.redraw();
    h.key(Key::Character("B".into()), Modifiers::empty());
    assert!(original.exists(), "editing a title must not rename before commit");
    assert!(!renamed.exists(), "title changes are committed on blur, not on every keystroke");

    h.root.focus_on(Some(body.id()));
    let signal = h.deliver();
    assert!(signal.title, "the host must refresh the Windows title on title-field blur");
    assert!(renamed.is_file(), "leaving the title field must persist the new filename");
    assert!(!original.exists(), "old filename must be gone after verified File Gate rename");
    assert!(h.app.note.as_ref().is_some_and(|note| note.is(&renamed)));
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(renamed.as_path()));
    assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "текст");
    assert!(h.app.popup.is_none());

    let mut h = h.restarted(1.0);
    h.app.open(&mut h.root, renamed.clone());
    assert!(h.app.open_error.is_none(), "blur-renamed file must reopen after restart");
    assert_eq!(h.body_text(), "текст");
}

#[test]
fn renaming_from_tab_title_enter_updates_file_tree_note_and_restart() {
    let mut h = Harness::new();
    let original = h.vault.path().join("a.md");
    let renamed = h.vault.path().join("B.md");
    h.app.open(&mut h.root, original.clone());
    h.app.start_tab_rename(&mut h.root, &original);
    let field = h.app.rename_field.expect("tab must expose the active rename editor");
    assert_eq!(h.root.focused_widget(), Some(field.id()));
    let _ = h.root.redraw();
    h.key(Key::Character("B".into()), Modifiers::empty());
    let signal = h.key(Key::Named(NamedKey::Enter), Modifiers::empty());
    assert!(signal.title, "the host must refresh the Windows title on tab-header Enter");

    assert!(renamed.is_file(), "tab-header Enter must rename the real file");
    assert!(!original.exists());
    assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "текст");
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(renamed.as_path()));
    assert!(h.app.note.as_ref().is_some_and(|note| note.is(&renamed)));
    assert!(h.app.open_error.is_none());
    assert!(h.app.popup.is_none());

    let mut h = h.restarted(1.0);
    h.app.open(&mut h.root, renamed.clone());
    assert!(h.app.open_error.is_none(), "tab-renamed file must reopen after restart");
    assert_eq!(h.body_text(), "текст");
}

#[test]
fn sidebar_rename_of_active_note_requests_window_title_refresh() {
    use crate::ui::editor::TextAction;
    let mut h = Harness::new();
    let from = h.vault.path().join("a.md");
    let renamed = h.vault.path().join("FromTree.md");
    h.app.open(&mut h.root, from.clone());
    h.app.tree.start_rename(&mut h.root, from.clone());
    let field = h.app.tree.rename_field().expect("sidebar title editor");
    let signal = h.act(field, TextAction::Entered("FromTree".to_owned()));

    assert!(signal.title, "sidebar-initiated active rename must refresh OS window title");
    assert!(renamed.is_file());
    assert!(!from.exists());
    assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "текст");
    assert!(h.app.note.as_ref().is_some_and(|note| note.is(&renamed)));
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(renamed.as_path()));
}

#[test]
fn core_relocation_event_requests_window_title_refresh() {
    let mut h = Harness::new();
    let from = h.vault.path().join("a.md");
    let to = h.vault.path().join("MovedExternally.md");
    h.app.open(&mut h.root, from.clone());
    aquilum_core::files::gate::rename(&h.app.workspace.core, &from, &to).unwrap();
    let retitle = h.app.on_core_events(&mut h.root);

    assert!(retitle, "Core relocation event must refresh the OS title");
    assert!(to.is_file());
    assert!(!from.exists());
    assert_eq!(std::fs::read_to_string(&to).unwrap(), "текст");
    assert!(h.app.note.as_ref().is_some_and(|note| note.is(&to)));
}

#[test]
fn normalization_equivalent_title_is_noop_without_an_error_or_disk_move() {
    let mut h = Harness::new();
    let original = h.write_note("Мои\u{306} заметки.md", "Исходные байты");
    let composed = h.vault.path().join("Мой заметки.md");
    let original_bytes = std::fs::read(&original).unwrap();
    assert_ne!(original, composed, "different Unicode spellings must be exercised");
    assert!(aquilum_core::search::paths::same_path(&original, &composed));

    h.app.open(&mut h.root, original.clone());
    assert!(crate::file_ops::rename_target(&original, "Мой заметки").is_none(),
        "title blur and Enter must not invent a Unicode-only rename");
    h.app.rename_file(&mut h.root, original.clone(), composed);

    assert!(h.app.popup.is_none(), "equivalent path must not show cross-vault failure");
    assert!(h.app.note.as_ref().is_some_and(|note| note.is(&original)));
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(original.as_path()));
    assert_eq!(std::fs::read(&original).unwrap(), original_bytes);
}

#[test]
fn case_only_title_change_is_still_a_requested_rename() {
    let from = std::path::Path::new("vault/Note.md");
    let (to, label) = crate::file_ops::rename_target(from, "note")
        .expect("case-only rename should still be offered to File Gate");
    assert_eq!(to, std::path::Path::new("vault/note.md"));
    assert_eq!(label, "note");
}

#[test]
fn overlapping_registered_vaults_still_block_a_real_rename() {
    let mut h = Harness::new();
    let nested = h.vault.path().join("Nested");
    std::fs::create_dir_all(&nested).unwrap();
    h.app.workspace.core.ui_state.resolve_workspace(nested.to_str().unwrap(), 2).unwrap();
    let original = h.write_note("Nested/Мои\u{306} заметки.md", "Неприкосновенный текст");
    let desired = nested.join("Другое имя.md");

    let result = aquilum_core::files::gate::rename(&h.app.workspace.core, &original, &desired);
    assert!(matches!(&result, Err(aquilum_core::files::error::FileCommandError::Task { message })
        if message.contains("Cross-workspace")), "expected cross-vault hold: {result:?}");
    assert_eq!(std::fs::read_to_string(&original).unwrap(), "Неприкосновенный текст");
    assert!(!desired.exists());
    assert!(crate::file_ops::rename_target(&original, "Другое имя").is_some(),
        "real moves must still pass through File Gate; do not discard them as no-op");

    h.app.open(&mut h.root, original.clone());
    h.app.rename_file(&mut h.root, original.clone(), original.clone());
    assert!(h.app.popup.is_none(), "an unchanged title must never enter cross-vault admission");
    h.app.rename_file(&mut h.root, original.clone(), nested.join("Мой заметки.md"));
    assert!(h.app.popup.is_none(), "normalization-only Enter must not hit the cross-vault gate");
    assert!(h.app.note.as_ref().is_some_and(|note| note.is(&original)));
    assert_eq!(std::fs::read_to_string(&original).unwrap(), "Неприкосновенный текст");
}

#[test]
fn sidebar_unicode_normalization_enter_stays_noop_with_nested_vault() {
    use crate::ui::editor::TextAction;

    let mut h = Harness::new();
    let nested = h.vault.path().join("Nested");
    std::fs::create_dir_all(&nested).unwrap();
    h.app
        .workspace
        .core
        .ui_state
        .resolve_workspace(nested.to_str().unwrap(), 2)
        .unwrap();

    let original = h.write_note("Nested/Мои\u{306} заметки.md", "Текст сохранён");
    let original_bytes = std::fs::read(&original).unwrap();
    let normalized = nested.join("Мой заметки.md");
    h.app.open(&mut h.root, original.clone());
    h.app.tree.start_rename(&mut h.root, original.clone());
    let field = h.app.tree.rename_field().expect("sidebar rename field");
    let _ = h.act(field, TextAction::Entered("Мой заметки".to_owned()));

    assert!(h.app.popup.is_none(), "equivalent Enter must not reach File Gate");
    assert!(original.is_file());
    assert!(!normalized.exists(), "no new Unicode spelling must be written");
    assert_eq!(std::fs::read(&original).unwrap(), original_bytes);
    assert!(h.app.note.as_ref().is_some_and(|note| note.is(&original)));
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(original.as_path()));
}
