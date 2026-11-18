use super::*;
use aquilum_core::learning::state::StudyStore;
use crate::ui::dialog::DialogAnswer;
use crate::ui::editor::TextAction;
use crate::ui::menu::MenuChoice;

#[test]
fn lang02_named_collection_native_gui_persists_without_mutating_markdown() {
    let mut h = Harness::new();
    let path = h.vault.path().join("a.md");
    let markdown = "| Term | Translation |\n| --- | --- |\n| star | звезда |";
    assert!(h.app.open(&mut h.root, path.clone()), "test fixture note must be opened");
    h.replace_body(markdown);
    h.save_now();
    let saved = std::fs::read(&path).unwrap();
    assert!(h.app.note.as_ref().is_some_and(|n| n.clean_cache_hash().is_some()));
    let body = h.app.note_ids.as_ref().unwrap().body;
    let inside_table = markdown.find("star").unwrap();
    h.root.focus_on(Some(body.id()));
    body.edit(&mut h.root, |mut editor| {
        crate::ui::editor::TextEditor::select_byte_range(&mut editor, inside_table, inside_table);
    });
    // A source-caret selection does NOT open the native table session.
    // In production that session is established by an actual pointer press.
    h.settle();
    let (x, y) = {
        let widget = body.get(&h.root).unwrap();
        let rect = widget.ctx().bounding_box();
        let table_top = widget.inner().top_of(0).expect("table must be laid out");
        (rect.x0 + 70.0, rect.y0 + table_top + 45.0)
    };
    h.click(x, y);
    let snapshot = body.get(&h.root).unwrap().inner().study_table_snapshot();
    assert!(snapshot.is_ok(), "native pointer must activate a table, got {snapshot:?}");

    h.app.open_study_add(&mut h.root);
    let question = match &h.app.popup {
        Some(Popup::StudyAddQuestion { layer, .. }) => *layer,
        _ => panic!("saved table should offer the study-column mapping menu"),
    };
    h.act(question, MenuChoice::Item(0));
    let answer = match &h.app.popup {
        Some(Popup::StudyAddAnswer { layer, .. }) => *layer,
        _ => panic!("choosing question must open answer mapping"),
    };
    h.act(answer, MenuChoice::Item(0));

    let (group_menu, new_index) = match &h.app.popup {
        Some(Popup::StudyAddCollection { layer, choices, .. }) => (
            *layer,
            choices.iter().position(|(id, _)| id.is_empty()).unwrap(),
        ),
        _ => panic!("table columns should lead to collection choice"),
    };
    h.act(group_menu, MenuChoice::Item(new_index));
    let field = match &h.app.popup {
        Some(Popup::StudyNewCollection { field, .. }) => field.id(),
        _ => panic!("New collection must open an editable native dialog"),
    };
    h.act(field, TextAction::Changed("  Физика / термины  ".into()));
    let dialog = match &h.app.popup {
        Some(Popup::StudyNewCollection { dialog, .. }) => dialog.layer,
        _ => panic!("edited new collection must keep the dialog open"),
    };
    h.act(dialog, DialogAnswer::Confirm);

    let db = h.app.workspace.data_dir.join("language-study.sqlite");
    let store = StudyStore::open(&db).unwrap();
    let data = store.load().unwrap();
    let group = data.state.registry.collections.iter()
        .find(|c| c.name == "Физика / термины")
        .expect("native dialog should create the named group");
    assert_eq!(group.members.len(), 1);
    assert_eq!(data.state.registry.tables.len(), 1);
    assert_eq!(std::fs::read(&path).unwrap(), saved, "binding changed Markdown");

    drop(store);
    let h = h.restarted(1.0);
    let restored = StudyStore::open(&h.app.workspace.data_dir.join("language-study.sqlite"))
        .unwrap().load().unwrap();
    assert!(restored.state.registry.collections.iter()
        .any(|c| c.name == "Физика / термины" && c.members.len() == 1));
    assert_eq!(std::fs::read(&path).unwrap(), saved, "restart changed Markdown");
    eprintln!("AKULA_LANG02_NATIVE_JOURNEY_PASS=columns,create,sqlite,restart,source_unchanged");
}
