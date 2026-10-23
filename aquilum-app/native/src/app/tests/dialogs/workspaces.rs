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
