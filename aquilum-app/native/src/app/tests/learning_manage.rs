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
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
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
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    eprintln!("AKULA_LANG02_STALE_PASS=revision_guard,source_unchanged");

    // The same session must observe the externally committed name after the
    // stale delete notice is dismissed; a fresh user confirmation can then
    // delete this collection without damaging source bindings or Markdown.
    assert!(matches!(&h.app.popup, Some(Popup::StudyPreview { .. })),
        "stale delete must leave a native conflict notice");
    notice_close(&mut h);
    h.app.open_study_manage(&mut h.root);
    let fresh_menu = match &h.app.popup {
        Some(Popup::StudyManage { layer, revision, actions }) => {
            assert_eq!(*revision, latest.revision);
            assert_eq!(actions.len(), 2);
            assert_eq!(actions[1].0, "botany");
            assert_eq!(actions[1].1, "Botany");
            assert!(actions[1].2);
            *layer
        }
        _ => panic!("same-session delete menu must reopen after stale conflict"),
    };
    h.act(fresh_menu, MenuChoice::Item(1));
    let fresh_delete = match &h.app.popup {
        Some(Popup::StudyDelete { dialog, revision, .. }) => {
            assert_eq!(*revision, latest.revision);
            dialog.layer
        }
        _ => panic!("fresh delete needs native user confirmation"),
    };
    h.act(fresh_delete, DialogAnswer::Confirm);
    let deleted = StudyStore::open(&db).unwrap().load().unwrap();
    assert_eq!(deleted.revision, latest.revision + 1);
    assert!(deleted.state.registry.collections.is_empty());
    assert_eq!(deleted.state.registry.tables, latest.state.registry.tables);
    assert_eq!(deleted.state.anchors, latest.state.anchors);
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    eprintln!("AKULA_LANG02_STALE_DELETE_SAME_SESSION_PASS=notice_closed,new_revision,source_unchanged");
}

#[test]
fn lang02_stale_rename_dialog_refuses_concurrent_update_without_losing_sources() {
    let mut h = Harness::new();
    let source = h.vault.path().join("a.md");
    let source_bytes = std::fs::read(&source).unwrap();
    let db = seed_group(&mut h);

    // Open a real native rename modal at the current collection revision.
    h.app.open_study_manage(&mut h.root);
    let layer = match &h.app.popup {
        Some(Popup::StudyManage { layer, actions, .. }) => {
            assert_eq!(actions[0].0, "botany");
            assert!(!actions[0].2, "first action should rename");
            *layer
        }
        _ => panic!("collection management must be visible"),
    };
    h.act(layer, MenuChoice::Item(0));
    let field = match &h.app.popup {
        Some(Popup::StudyRename { field, .. }) => field.id(),
        _ => panic!("native rename dialog must be open"),
    };
    h.act(field, TextAction::Changed("Stale dialog value".into()));
    let stale_dialog = match &h.app.popup {
        Some(Popup::StudyRename { dialog, .. }) => dialog.layer,
        _ => panic!("native rename modal must preserve its initial revision"),
    };

    // Another writer commits while the original dialog remains open.
    // Only the temporary app-owned SQLite is touched, never user data.
    let mut writer = StudyStore::open(&db).unwrap();
    let initial_revision = writer.load().unwrap().revision;
    let committed_revision = writer.rename_collection(initial_revision, "botany", "Concurrent winner")
        .expect("concurrent synthetic writer must succeed");
    let before_stale = writer.load().unwrap();
    assert_eq!(before_stale.revision, committed_revision);
    assert_eq!(before_stale.state.registry.collections[0].name, "Concurrent winner");

    h.act(stale_dialog, DialogAnswer::Confirm);
    assert!(matches!(&h.app.popup, Some(Popup::StudyPreview { .. })),
        "stale native rename must show a notice rather than succeeding");
    let after_stale = StudyStore::open(&db).unwrap().load().unwrap();
    assert_eq!(after_stale.revision, committed_revision,
        "rejected rename must not advance SQLite revision");
    assert_eq!(after_stale.state.registry.collections[0].id, "botany");
    assert_eq!(after_stale.state.registry.collections[0].name, "Concurrent winner",
        "stale native dialog must never overwrite concurrent metadata");
    assert_eq!(after_stale.state.registry.tables, before_stale.state.registry.tables,
        "stale rename must not modify source mappings");
    assert_eq!(after_stale.state.anchors, before_stale.state.anchors,
        "stale rename must not modify anchors");
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes,
        "stale rename must not modify Markdown");
    eprintln!("AKULA_LANG02_STALE_RENAME_PASS=dialog_cas,concurrent_winner,source_unchanged");

    // A conflict refusal must not leave a stale popup or stale collection
    // snapshot behind: the very same native session must list the external
    // winner and accept a new confirmation at the updated revision.
    notice_close(&mut h);
    h.app.open_study_manage(&mut h.root);
    let current_layer = match &h.app.popup {
        Some(Popup::StudyManage { layer, revision, actions }) => {
            assert_eq!(*revision, committed_revision);
            assert_eq!(actions.len(), 2);
            assert_eq!(actions[0].0, "botany");
            assert_eq!(actions[0].1, "Concurrent winner");
            assert!(!actions[0].2);
            *layer
        }
        _ => panic!("same-session management must reopen after stale rename refusal"),
    };
    h.act(current_layer, MenuChoice::Item(0));
    let field = match &h.app.popup {
        Some(Popup::StudyRename { field, revision, .. }) => {
            assert_eq!(*revision, committed_revision);
            field.id()
        }
        _ => panic!("fresh rename must expose a native editable dialog"),
    };
    h.act(field, TextAction::Changed("Current winner".into()));
    let fresh_dialog = match &h.app.popup {
        Some(Popup::StudyRename { dialog, .. }) => dialog.layer,
        _ => panic!("fresh rename confirmation missing after conflict"),
    };
    h.act(fresh_dialog, DialogAnswer::Confirm);
    let after_recovery = StudyStore::open(&db).unwrap().load().unwrap();
    assert_eq!(after_recovery.revision, committed_revision + 1);
    assert_eq!(after_recovery.state.registry.collections[0].id, "botany");
    assert_eq!(after_recovery.state.registry.collections[0].name, "Current winner");
    assert_eq!(after_recovery.state.registry.tables, before_stale.state.registry.tables);
    assert_eq!(after_recovery.state.anchors, before_stale.state.anchors);
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    eprintln!("AKULA_LANG02_STALE_RENAME_SAME_SESSION_PASS=notice_closed,new_revision,source_unchanged");
}
