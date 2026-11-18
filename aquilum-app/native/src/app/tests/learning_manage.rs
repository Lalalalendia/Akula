use super::*;
use aquilum_core::learning::{ColumnMapping};
use aquilum_core::learning::state::StudyStore;
use aquilum_core::learning::table_anchors::NoteKey;
use crate::ui::dialog::DialogAnswer;
use crate::ui::editor::TextAction;
use crate::ui::menu::MenuChoice;

fn seed_group(h: &mut Harness) -> std::path::PathBuf {
    let path = h.app.workspace.data_dir.join("language-study.sqlite");
    let mut store = StudyStore::open(&path).unwrap();
    let key = NoteKey { vault_id: "vault-1".into(), note_id: "note-1".into() };
    let rows = [vec![
        vec!["Word".into(), "Meaning".into()],
        vec!["root".into(), "корень".into()],
    ]];
    store.bind(0, &key, &rows, 0,
        ColumnMapping { question: 0, answers: vec![1], context: None },
        "botany", "Ботаника", "table-1").unwrap();
    path
}

fn notice_close(h: &mut Harness) {
    let dialog = match &h.app.popup {
        Some(Popup::StudyPreview { dialog, .. }) => dialog.layer,
        _ => panic!("study operation should show a native result dialog"),
    };
    h.act(dialog, DialogAnswer::Cancel);
}

#[test]
fn lang02_rename_and_delete_require_native_confirmation_and_preserve_note() {
    let mut h = Harness::new();
    let source = h.vault.path().join("a.md");
    let source_bytes = std::fs::read(&source).unwrap();
    let db = seed_group(&mut h);

    h.app.open_study_manage(&mut h.root);
    let layer = match &h.app.popup {
        Some(Popup::StudyManage { layer, actions, .. }) => {
            assert_eq!(actions.len(), 2);
            assert_eq!(actions[0].0, "botany");
            assert!(!actions[0].2);
            assert!(actions[1].2);
            *layer
        }
        _ => panic!("named collections should have a manage menu"),
    };
    h.act(layer, MenuChoice::Item(0));

    let field = match &h.app.popup {
        Some(Popup::StudyRename { field, .. }) => field.id(),
        _ => panic!("rename should open a native name dialog"),
    };
    h.act(field, TextAction::Changed("  Биология  ".into()));
    let rename_dialog = match &h.app.popup {
        Some(Popup::StudyRename { dialog, .. }) => dialog.layer,
        _ => panic!("rename dialog should stay open for confirmation"),
    };
    h.act(rename_dialog, DialogAnswer::Confirm);
    let state = StudyStore::open(&db).unwrap().load().unwrap();
    assert_eq!(state.state.registry.collections[0].id, "botany");
    assert_eq!(state.state.registry.collections[0].name, "Биология");
    assert_eq!(state.state.registry.tables.len(), 1);
    notice_close(&mut h);

    h.app.open_study_manage(&mut h.root);
    let layer = match &h.app.popup {
        Some(Popup::StudyManage { layer, .. }) => *layer,
        _ => panic!("renamed group must still be manageable"),
    };
    h.act(layer, MenuChoice::Item(1));
    let deletion = match &h.app.popup {
        Some(Popup::StudyDelete { dialog, .. }) => dialog.layer,
        _ => panic!("delete must open a real confirmation"),
    };
    h.act(deletion, DialogAnswer::Cancel);
    assert_eq!(StudyStore::open(&db).unwrap().load().unwrap().state.registry.collections.len(), 1);

    h.app.open_study_manage(&mut h.root);
    let layer = match &h.app.popup {
        Some(Popup::StudyManage { layer, .. }) => *layer,
        _ => panic!("cancelled deletion must keep the collection available"),
    };
    h.act(layer, MenuChoice::Item(1));
    let deletion = match &h.app.popup {
        Some(Popup::StudyDelete { dialog, .. }) => dialog.layer,
        _ => panic!("delete must require a fresh confirmation"),
    };
    h.act(deletion, DialogAnswer::Confirm);
    let state = StudyStore::open(&db).unwrap().load().unwrap();
    assert!(state.state.registry.collections.is_empty());
    assert_eq!(state.state.registry.tables.len(), 1, "do not erase source mappings");
    assert_eq!(state.state.anchors.len(), 1, "do not erase table anchors");
    assert_eq!(std::fs::read(source).unwrap(), source_bytes);
    eprintln!("AKULA_LANG02_MANAGE_PASS=rename,confirm_cancel,delete,source_unchanged");
}

#[test]
fn lang02_stale_collection_menu_refuses_delete_without_changing_sources() {
    let mut h = Harness::new();
    let source = h.vault.path().join("a.md");
    let source_bytes = std::fs::read(&source).unwrap();
    let db = seed_group(&mut h);
    h.app.open_study_manage(&mut h.root);
    let layer = match &h.app.popup {
        Some(Popup::StudyManage { layer, .. }) => *layer,
        _ => panic!("collection must be listed"),
    };

    let mut writer = StudyStore::open(&db).unwrap();
    let rev = writer.load().unwrap().revision;
    writer.rename_collection(rev, "botany", "Botany").unwrap();
    h.act(layer, MenuChoice::Item(1));
    let dialog = match &h.app.popup {
        Some(Popup::StudyDelete { dialog, .. }) => dialog.layer,
        _ => panic!("confirmation is required"),
    };
    h.act(dialog, DialogAnswer::Confirm);
    let latest = StudyStore::open(&db).unwrap().load().unwrap();
    assert_eq!(latest.state.registry.collections[0].name, "Botany");
    assert_eq!(latest.state.registry.collections[0].id, "botany");
    assert_eq!(std::fs::read(source).unwrap(), source_bytes);
    eprintln!("AKULA_LANG02_STALE_PASS=revision_guard,source_unchanged");
}
