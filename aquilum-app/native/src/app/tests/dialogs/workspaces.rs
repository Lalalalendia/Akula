use super::*;
use crate::ui::dialog::DialogAnswer;
use crate::ui::widgets::Pressed;
use crate::ui::workspaces::RowAction;

#[test]
fn closing_the_only_workspace_returns_to_no_folder_and_preserves_notes() {
    let mut h = Harness::new();
    let path = h.vault.path().to_path_buf();
    let note = path.join("a.md");
    let before = std::fs::read(&note).unwrap();
    assert!(h.app.open(&mut h.root, note.clone()));
    h.app.open_workspaces(&mut h.root);
    let button = match &h.app.popup {
        Some(Popup::Workspaces(state)) => state.dialog().close_active.expect("close base button"),
        _ => panic!("expected workspace picker"),
    };
    let outcome = h.send(button, Pressed);
    assert!(outcome.title, "closing a vault updates the window title");
    assert!(h.app.tree.root().is_none());
    assert!(h.app.workspace.core.search.active_root().is_none());
    assert!(h.app.workspace.root.is_none());
    assert!(h.app.tabs.active().path.is_none());
    assert!(h.app.note.is_none());
    assert!(h.app.workspace.core.active_note.get().is_none());
    assert_eq!(std::fs::read(&note).unwrap(), before);
    assert!(h.app.workspace.core.ui_state.list_workspaces().unwrap()
        .iter().any(|w| crate::ui::workspaces::same_path(std::path::Path::new(&w.path), &path)));
    assert!(h.app.switch_workspace(&mut h.root, path).title);
    assert!(h.app.tree.root().is_some());
    assert_eq!(std::fs::read(&note).unwrap(), before);
}

#[test]
fn forgetting_current_workspace_closes_it_but_never_deletes_its_directory() {
    let mut h = Harness::new();
    let path = h.vault.path().to_path_buf();
    let note = path.join("a.md");
    let before = std::fs::read(&note).unwrap();
    h.app.open_workspaces(&mut h.root);
    let row = match &h.app.popup {
        Some(Popup::Workspaces(state)) => state.dialog().rows.iter()
            .find(|(_, p, _)| crate::ui::workspaces::same_path(p, &path))
            .map(|(id, _, _)| *id).expect("active workspace row"),
        _ => panic!("expected workspace picker"),
    };
    h.send(row, RowAction::Forget);
    let layer = match &h.app.popup {
        Some(Popup::Workspaces(state)) => state.forget_layer().expect("confirmation"),
        _ => panic!("expected confirmation"),
    };
    h.send(layer, DialogAnswer::Confirm);
    assert!(h.app.workspace.root.is_none());
    assert!(h.app.tree.root().is_none());
    assert!(h.app.workspace.core.ui_state.list_workspaces().unwrap()
        .iter().all(|w| !crate::ui::workspaces::same_path(std::path::Path::new(&w.path), &path)));
    assert_eq!(std::fs::read(&note).unwrap(), before);
}

#[test]
fn unsaved_failure_must_keep_current_workspace_and_editor_open() {
    let mut h = Harness::new();
    let path = h.vault.path().to_path_buf();
    let note = path.join("a.md");
    assert!(h.app.open(&mut h.root, note.clone()));
    h.app.note.as_mut().unwrap().save_error = Some("manual review required".to_owned());
    assert!(!h.app.close_current_workspace(&mut h.root).title);
    assert!(h.app.workspace.root.as_deref() == Some(path.as_path()));
    assert!(h.app.note.as_ref().is_some_and(|n| n.is(&note)));
    assert_eq!(h.app.workspace.core.active_note.get().as_deref(), Some(note.as_path()));
}

#[test]
fn switching_workspace_saves_pending_text_before_leaving() {
    let mut h = Harness::new();
    let note = h.vault.path().join("a.md");
    let next = crate::test_dir::tempdir();
    assert!(h.app.open(&mut h.root, note.clone()));
    h.replace_body("правка перед сменой папки 😀");
    assert_ne!(std::fs::read_to_string(&note).unwrap(), h.body_text());

    assert!(h.app.switch_workspace(&mut h.root, next.path().to_path_buf()).title);
    assert_eq!(std::fs::read_to_string(&note).unwrap(), "правка перед сменой папки 😀");
    assert_eq!(h.app.tree.root(), Some(next.path()));
    assert!(h.app.note.is_none());
    assert!(h.app.popup.is_none());
}

#[test]
fn switching_workspace_keeps_unsaved_editor_and_remembered_folder_on_failure() {
    let mut h = Harness::new();
    let vault = h.vault.path().to_path_buf();
    let note = vault.join("a.md");
    let backup = vault.join("original.txt");
    let next = crate::test_dir::tempdir();
    assert!(h.app.open(&mut h.root, note.clone()));
    let editor = h.app.editor_id().unwrap();
    let tab = h.app.tabs.active_id();
    h.replace_body("эта правка должна остаться в редакторе");
    // A directory at the file's path makes the real File Gate save fail,
    // including when tests run with administrator/root permissions.
    std::fs::rename(&note, &backup).unwrap();
    std::fs::create_dir(&note).unwrap();

    assert!(!h.app.switch_workspace(&mut h.root, next.path().to_path_buf()).title);
    assert_eq!(h.app.tree.root(), Some(vault.as_path()));
    assert_eq!(h.app.workspace.root.as_deref(), Some(vault.as_path()));
    assert_eq!(h.app.tabs.active_id(), tab);
    assert_eq!(h.app.editor_id(), Some(editor));
    assert_eq!(h.body_text(), "эта правка должна остаться в редакторе");
    assert!(h.app.note.as_ref().unwrap().save_error.is_some());
    assert!(h.app.save_error_ids.is_some());
    assert_eq!(h.app.workspace.core.active_note.get().as_deref(), Some(note.as_path()));
    let known = h.app.workspace.core.ui_state.list_workspaces().unwrap();
    assert_eq!(known[0].path, vault.to_string_lossy());
    assert!(!known.iter().any(|w| crate::ui::workspaces::same_path(std::path::Path::new(&w.path), next.path())));
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), "текст");

    // Restore the unchanged original, then explicitly retry the pending edit.
    std::fs::remove_dir(&note).unwrap();
    std::fs::rename(&backup, &note).unwrap();
    h.app.note.as_mut().unwrap().retry();
    assert!(h.app.switch_workspace(&mut h.root, next.path().to_path_buf()).title);
    assert_eq!(std::fs::read_to_string(note).unwrap(), "эта правка должна остаться в редакторе");
}

#[test]
fn switching_workspace_keeps_the_current_folder_and_both_conflicting_versions() {
    let mut h = Harness::new();
    let vault = h.vault.path().to_path_buf();
    let note = vault.join("a.md");
    let next = crate::test_dir::tempdir();
    assert!(h.app.open(&mut h.root, note.clone()));
    h.replace_body("моя версия 😀");
    std::fs::write(&note, "внешняя версия").unwrap();

    assert!(!h.app.switch_workspace(&mut h.root, next.path().to_path_buf()).title);
    assert_eq!(h.app.tree.root(), Some(vault.as_path()));
    assert_eq!(h.body_text(), "моя версия 😀");
    assert!(h.app.note.as_ref().unwrap().save_error.is_some());
    assert_eq!(std::fs::read_to_string(&note).unwrap(), "внешняя версия");
    let copies: Vec<_> = std::fs::read_dir(&vault).unwrap().flatten()
        .map(|entry| entry.path())
        .filter(|path| path.to_string_lossy().contains("конфликт"))
        .collect();
    assert_eq!(copies.len(), 1);
    assert!(std::fs::read_to_string(&copies[0]).unwrap().contains("моя версия 😀"));
}

#[test]
fn selecting_the_current_workspace_does_not_discard_or_flush_the_editor() {
    let mut h = Harness::new();
    let vault = h.vault.path().to_path_buf();
    let note = vault.join("a.md");
    assert!(h.app.open(&mut h.root, note.clone()));
    h.replace_body("несохранённый текст");
    let editor = h.app.editor_id().unwrap();
    h.app.open_workspaces(&mut h.root);

    assert!(!h.app.switch_workspace(&mut h.root, vault).title);
    assert!(h.app.popup.is_none());
    assert_eq!(h.app.editor_id(), Some(editor));
    assert_eq!(h.body_text(), "несохранённый текст");
    assert_eq!(std::fs::read_to_string(note).unwrap(), "текст");
}
