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
    h.deliver();
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
    h.key(Key::Named(NamedKey::Enter), Modifiers::empty());

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
