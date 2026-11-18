use super::*;
use super::learning_multi::bind_fixture;
use aquilum_core::text::annotation_store::AnnotationStore;
use crate::ui::editor::TextEditor;
use crate::ui::editor::resolve::Request;
use crate::ui::menu::MenuChoice;
use masonry::kurbo::Point;

fn create_highlight(h: &mut Harness) {
    let ids = h.app.note_ids.as_ref().expect("active note");
    let body = ids.body.get(&h.root).unwrap().inner().text();
    let offset = body.find("eat").expect("fixture word eat");
    ids.body.edit(&mut h.root, |mut editor| {
        TextEditor::select_byte_range(&mut editor, offset, offset + 3);
    });
    h.app.open_editor_context_menu(&mut h.root, Point::ORIGIN, true, None);
    let layer = match &h.app.popup {
        Some(Popup::EditorMenu { layer, .. }) => *layer,
        _ => panic!("editor menu missing"),
    };
    h.act(layer, MenuChoice::Item(1));
    let layer = match &h.app.popup {
        Some(Popup::AnnotationColors { layer, .. }) => *layer,
        _ => panic!("annotation palette missing"),
    };
    h.act(layer, MenuChoice::Item(0));
}

fn clicked_request(h: &Harness) -> Request {
    let store = AnnotationStore::open(
        &h.app.workspace.data_dir.join("text-annotations.sqlite"),
    ).unwrap();
    let range = &store.load().unwrap().records[0].anchor;
    Request::AnnotationPressed {
        editor: h.app.note_ids.as_ref().unwrap().body.id(),
        range: range.byte_start..range.byte_end,
    }
}

#[test]
fn lang03_native_click_shows_note_without_mutating_source_or_cas_revision() {
    let mut h = Harness::new();
    let (a, _) = bind_fixture(&mut h);
    let before = std::fs::read(&a).unwrap();
    assert!(h.app.open(&mut h.root, a.clone()));
    create_highlight(&mut h);
    let path = h.app.workspace.data_dir.join("text-annotations.sqlite");
    let mut store = AnnotationStore::open(&path).unwrap();
    let mut snapshot = store.load().unwrap();
    snapshot.records[0].explanation = Some("Meaning from source note".into());
    assert_eq!(store.replace(snapshot.revision, &snapshot.records), Ok(2));
    let request = clicked_request(&h);
    assert!(!h.app.on_editor_request(&mut h.root, request));
    assert!(matches!(h.app.popup, Some(Popup::StudyPreview { .. })),
        "clicked colored text must open a read-only preview");
    assert_eq!(store.load().unwrap().revision, 2);
    assert_eq!(std::fs::read(&a).unwrap(), before);
    eprintln!("AKULA_LANG03_CLICK_PASS=current_source,read_only");
}

#[test]
fn lang03_native_click_on_stale_source_never_reveals_note() {
    let mut h = Harness::new();
    let (a, _) = bind_fixture(&mut h);
    assert!(h.app.open(&mut h.root, a.clone()));
    create_highlight(&mut h);
    let request = clicked_request(&h);
    std::fs::write(&a, "| Q | A |\n| --- | --- |\n| ran | чужое значение |\n").unwrap();
    assert!(!h.app.on_editor_request(&mut h.root, request));
    assert!(h.app.popup.is_none(), "stale source must not reveal annotation");
    let store = AnnotationStore::open(
        &h.app.workspace.data_dir.join("text-annotations.sqlite"),
    ).unwrap();
    assert_eq!(store.load().unwrap().revision, 1);
}
