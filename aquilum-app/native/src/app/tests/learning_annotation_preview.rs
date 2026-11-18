use super::*;
use super::learning_multi::bind_fixture;
use aquilum_core::text::annotation_store::AnnotationStore;
use crate::ui::editor::TextEditor;
use crate::ui::menu::MenuChoice;
use masonry::kurbo::Point;

fn select_eat(h: &mut Harness) {
    let ids = h.app.note_ids.as_ref().expect("open note");
    let text = ids.body.get(&h.root).unwrap().inner().text();
    let start = text.find("eat").expect("fixture has word");
    ids.body.edit(&mut h.root, |mut editor| {
        TextEditor::select_byte_range(&mut editor, start, start + 3);
    });
}

fn menu(h: &mut Harness) -> masonry::core::WidgetId {
    h.app.open_editor_context_menu(&mut h.root, Point::ORIGIN, true, None);
    match &h.app.popup {
        Some(Popup::EditorMenu { layer, commands }) => {
            assert!(matches!(commands.get(3), Some(
                super::super::editor_requests::EditorCommand::ViewExplanation
            )));
            *layer
        }
        _ => panic!("selection should expose context menu"),
    }
}

fn save_yellow_highlight(h: &mut Harness) {
    let layer = menu(h);
    h.act(layer, MenuChoice::Item(1));
    let layer = match &h.app.popup {
        Some(Popup::AnnotationColors { layer, .. }) => *layer,
        _ => panic!("palette must open"),
    };
    h.act(layer, MenuChoice::Item(0));
}

#[test]
fn lang03_read_only_preview_of_existing_explanation_keeps_source_and_revision() {
    let mut h = Harness::new();
    let (a, _) = bind_fixture(&mut h);
    let before = std::fs::read(&a).unwrap();
    assert!(h.app.open(&mut h.root, a.clone()));
    select_eat(&mut h);
    save_yellow_highlight(&mut h);

    let path = h.app.workspace.data_dir.join("text-annotations.sqlite");
    let mut store = AnnotationStore::open(&path).unwrap();
    let mut saved = store.load().unwrap();
    assert_eq!(saved.records.len(), 1);
    saved.records[0].explanation = Some("Past tense: ate".into());
    assert_eq!(store.replace(saved.revision, &saved.records).unwrap(), 2);

    let layer = menu(&mut h);
    h.act(layer, MenuChoice::Item(3));
    assert!(matches!(h.app.popup, Some(Popup::StudyPreview { .. })),
        "read-only preview should use existing notice dialog");
    assert_eq!(store.load().unwrap().revision, 2,
        "viewing must not write or advance CAS revision");
    assert_eq!(std::fs::read(&a).unwrap(), before);
    eprintln!("AKULA_LANG03_PREVIEW_PASS=authorized,read_only");
}

#[test]
fn lang03_read_only_preview_of_empty_explanation_never_creates_metadata() {
    let mut h = Harness::new();
    let (a, _) = bind_fixture(&mut h);
    assert!(h.app.open(&mut h.root, a));
    select_eat(&mut h);
    save_yellow_highlight(&mut h);

    let path = h.app.workspace.data_dir.join("text-annotations.sqlite");
    let store = AnnotationStore::open(&path).unwrap();
    assert!(store.load().unwrap().records[0].explanation.is_none());
    let layer = menu(&mut h);
    h.act(layer, MenuChoice::Item(3));
    assert!(matches!(h.app.popup, Some(Popup::StudyPreview { .. })));
    assert_eq!(store.load().unwrap().revision, 1);
}
