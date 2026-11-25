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
    h.act(question, MenuChoice::Hover(0));
    assert!(matches!(&h.app.popup, Some(Popup::StudyAddQuestion { layer, .. }) if *layer == question));
    h.act(question, MenuChoice::Item(0));
    let answer = match &h.app.popup {
        Some(Popup::StudyAddAnswer { layer, .. }) => *layer,
        _ => panic!("choosing question must open answer mapping"),
    };
    h.act(answer, MenuChoice::Hover(0));
    assert!(matches!(&h.app.popup, Some(Popup::StudyAddAnswer { layer, question: 0, .. }) if *layer == answer));
    h.act(answer, MenuChoice::Item(0));

    let (group_menu, new_index) = match &h.app.popup {
        Some(Popup::StudyAddCollection { layer, choices, .. }) => (
            *layer,
            choices.iter().position(|(id, _)| id.is_empty()).unwrap(),
        ),
        _ => panic!("table columns should lead to collection choice"),
    };
    h.act(group_menu, MenuChoice::Hover(new_index));
    assert!(matches!(&h.app.popup, Some(Popup::StudyAddCollection { layer, choices, .. })
        if *layer == group_menu && choices.get(new_index).is_some_and(|(id, _)| id.is_empty())));
    h.act(group_menu, MenuChoice::Item(new_index));
    let field = match &h.app.popup {
        Some(Popup::StudyNewCollection { field, .. }) => {
            assert!(field.get(&h.root).unwrap().inner().text().is_empty());
            field.id()
        }
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

#[test]
fn lang08_dirty_note_refuses_collection_bind_without_persisting_metadata() {
    let mut h = Harness::new();
    let path = h.vault.path().join("a.md");
    let saved_markdown = "| Term | Translation |\n| --- | --- |\n| star | star |";
    assert!(h.app.open(&mut h.root, path.clone()));
    h.replace_body(saved_markdown);
    h.save_now();
    let on_disk = std::fs::read(&path).unwrap();

    // Exercise a real editor modification, not a synthetic database write.
    let dirty_markdown = saved_markdown.replace("star | star", "moon | moon");
    h.replace_body(&dirty_markdown);
    assert!(h.app.note.as_ref().is_some_and(|n| n.clean_cache_hash().is_none()),
        "the edited note must still be unsaved");

    // Activate the live table by pointer, as in the positive native GUI journey.
    let body = h.app.note_ids.as_ref().unwrap().body;
    let inside_table = dirty_markdown.find("moon").unwrap();
    h.root.focus_on(Some(body.id()));
    body.edit(&mut h.root, |mut editor| {
        crate::ui::editor::TextEditor::select_byte_range(&mut editor, inside_table, inside_table);
    });
    h.settle();
    let (x, y) = {
        let widget = body.get(&h.root).unwrap();
        let rect = widget.ctx().bounding_box();
        let table_top = widget.inner().top_of(0).expect("table layout");
        (rect.x0 + 70.0, rect.y0 + table_top + 45.0)
    };
    h.click(x, y);
    assert!(body.get(&h.root).unwrap().inner().study_table_snapshot().is_ok(),
        "dirty note must still expose the current table through the live editor");
    assert!(h.app.note.as_ref().is_some_and(|n| n.clean_cache_hash().is_none()));

    h.app.open_study_add(&mut h.root);
    assert!(matches!(&h.app.popup, Some(Popup::StudyPreview { .. })),
        "dirty source must show a refusal notice, not a column or collection picker");
    let db = h.app.workspace.data_dir.join("language-study.sqlite");
    let store = StudyStore::open(&db).unwrap();
    let stored = store.load().unwrap();
    assert_eq!(stored.revision, 0, "dirty source must not increment SQLite revision");
    assert!(stored.state.registry.collections.is_empty());
    assert!(stored.state.registry.tables.is_empty());
    assert!(stored.state.anchors.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), on_disk,
        "refused collection binding must not modify the saved Markdown file");
    eprintln!("AKULA_LANG08_DIRTY_NATIVE_REFUSAL_PASS=editor_dirty,notice,no_metadata,markdown_unchanged");
}

#[test]
fn lang02_changed_table_during_collection_dialog_refuses_stale_bind() {
    let mut h = Harness::new();
    let path = h.vault.path().join("a.md");
    let original = "| Term | Translation |\n| --- | --- |\n| star | звезда |";
    assert!(h.app.open(&mut h.root, path.clone()));
    h.replace_body(original);
    h.save_now();
    let saved_bytes = std::fs::read(&path).unwrap();
    assert!(h.app.note.as_ref().is_some_and(|n| n.clean_cache_hash().is_some()));

    // Follow the actual native table pointer and column-mapping UI.
    let body = h.app.note_ids.as_ref().unwrap().body;
    let inside = original.find("star").unwrap();
    h.root.focus_on(Some(body.id()));
    body.edit(&mut h.root, |mut editor| {
        crate::ui::editor::TextEditor::select_byte_range(&mut editor, inside, inside);
    });
    h.settle();
    let (x, y) = {
        let widget = body.get(&h.root).unwrap();
        let rect = widget.ctx().bounding_box();
        let top = widget.inner().top_of(0).expect("fixture table layout");
        (rect.x0 + 70.0, rect.y0 + top + 45.0)
    };
    h.click(x, y);
    assert!(body.get(&h.root).unwrap().inner().study_table_snapshot().is_ok());
    h.app.open_study_add(&mut h.root);
    let question = match &h.app.popup {
        Some(Popup::StudyAddQuestion { layer, .. }) => *layer,
        _ => panic!("saved note must expose native question picker"),
    };
    h.act(question, MenuChoice::Item(0));
    let answer = match &h.app.popup {
        Some(Popup::StudyAddAnswer { layer, .. }) => *layer,
        _ => panic!("question choice must lead to answer picker"),
    };
    h.act(answer, MenuChoice::Item(0));
    let (collection_menu, new_index) = match &h.app.popup {
        Some(Popup::StudyAddCollection { layer, choices, .. }) => (
            *layer,
            choices.iter().position(|(id, _)| id.is_empty()).unwrap(),
        ),
        _ => panic!("column mapping must reach collection selection"),
    };
    h.act(collection_menu, MenuChoice::Item(new_index));
    let field = match &h.app.popup {
        Some(Popup::StudyNewCollection { field, .. }) => field.id(),
        _ => panic!("New collection must open native name dialog"),
    };
    h.act(field, TextAction::Changed("Старая таблица".into()));

    // Deliberately mutate the real underlying editor widget while its modal
    // is open. This is a stale-snapshot harness probe, not a normal user edit:
    // modal dispatch can suppress the note's dirty-state notification.
    // Compare the live parsed table with the captured table explicitly rather
    // than asserting a synthetic dirty-state transition.
    let changed = original.replace("star | звезда", "moon | луна");
    h.replace_body(&changed);
    assert_eq!(h.body_text(), changed);
    let current = body.get(&h.root).unwrap().inner().study_complete_note()
        .expect("modified editor must still contain a valid study table");
    let dialog = match &h.app.popup {
        Some(Popup::StudyNewCollection { dialog, pending, .. }) => {
            assert_ne!(current.selected, pending.table.selected,
                "stale-dialog fixture must differ from its saved table snapshot");
            dialog.layer
        }
        _ => panic!("stale create dialog unexpectedly disappeared before confirmation"),
    };
    h.act(dialog, DialogAnswer::Confirm);
    assert!(matches!(&h.app.popup, Some(Popup::StudyPreview { .. })),
        "stale source must refuse with native notice instead of saving membership");

    // The application may create an empty SQLite file when listing collection
    // choices; no metadata or Markdown side effect is permitted on refusal.
    let db = h.app.workspace.data_dir.join("language-study.sqlite");
    let snapshot = StudyStore::open(&db).unwrap().load().unwrap();
    assert_eq!(snapshot.revision, 0, "stale bind must not advance revision");
    assert!(snapshot.state.registry.collections.is_empty());
    assert!(snapshot.state.registry.tables.is_empty());
    assert!(snapshot.state.anchors.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), saved_bytes,
        "stale create dialog must not modify saved Markdown");
    eprintln!("AKULA_LANG02_CHANGED_DIALOG_PASS=stale_editor,notice,no_sqlite_write,source_unchanged");
}

#[test]
fn lang02_preview_menu_hover_preserves_snapshot_and_explicit_column_mapping() {
    use aquilum_core::learning::editor_snapshot::capture;
    let mut h = Harness::new();
    let rows = vec![vec!["Q".into(), "A".into()], vec!["star".into(), "answer".into()]];
    let snapshot = capture(&rows, 0..20, 20, false).unwrap();
    let path = h.vault.path().join("a.md");
    let original = std::fs::read(&path).unwrap();
    let db = h.app.workspace.data_dir.join("language-study.sqlite");
    let before = StudyStore::open(&db).unwrap().load().unwrap().revision;
    h.app.open_study_question(&mut h.root, snapshot.clone());
    let layer = match &h.app.popup {
        Some(Popup::StudyQuestion { layer, .. }) => *layer,
        _ => panic!("question menu missing"),
    };
    h.act(layer, MenuChoice::Hover(1));
    assert!(matches!(&h.app.popup, Some(Popup::StudyQuestion { layer: current, snapshot: retained })
        if *current == layer && retained == &snapshot));
    h.act(layer, MenuChoice::Item(1));
    let answer = match &h.app.popup {
        Some(Popup::StudyAnswer { layer, question: 1, snapshot: retained }) => {
            assert_eq!(retained, &snapshot);
            *layer
        }
        _ => panic!("reverse column choice must open the answer menu"),
    };
    h.act(answer, MenuChoice::Hover(0));
    assert!(matches!(&h.app.popup, Some(Popup::StudyAnswer { layer: current, question: 1, snapshot: retained })
        if *current == answer && retained == &snapshot));
    h.act(answer, MenuChoice::Item(0));
    assert!(matches!(&h.app.popup, Some(Popup::StudyPreview { copy: Some(value), .. }) if value == "answer\tstar"));
    assert_eq!(StudyStore::open(&db).unwrap().load().unwrap().revision, before);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    eprintln!("AKULA_REUSE14_STUDY_PREVIEW_PASS=hover,reverse_mapping,no_write");
}
