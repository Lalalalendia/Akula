use super::*;
use super::learning_multi::bind_fixture;
use aquilum_core::text::annotation_store::AnnotationStore;
use crate::ui::editor::TextEditor;
use crate::ui::menu::MenuChoice;
use masonry::kurbo::Point;

fn ranges(h: &Harness) -> Vec<std::ops::Range<usize>> {
    h.app.note_ids.as_ref().unwrap().body
        .get(&h.root).unwrap().inner().annotation_ranges_for_test()
}

fn save_highlight(h: &mut Harness) -> std::ops::Range<usize> {
    let ids = h.app.note_ids.as_ref().unwrap();
    let text = ids.body.get(&h.root).unwrap().inner().text();
    let start = text.find("eat").unwrap();
    let range = start..start + 3;
    ids.body.edit(&mut h.root, |mut editor| {
        TextEditor::select_byte_range(&mut editor, range.start, range.end);
    });
    h.app.open_editor_context_menu(&mut h.root, Point::ORIGIN, true, None);
    let layer = match &h.app.popup {
        Some(Popup::EditorMenu { layer, .. }) => *layer,
        _ => panic!("missing editor menu"),
    };
    h.act(layer, MenuChoice::Item(1));
    let layer = match &h.app.popup {
        Some(Popup::AnnotationColors { layer, .. }) => *layer,
        _ => panic!("missing palette"),
    };
    h.act(layer, MenuChoice::Item(2));
    range
}

#[test]
fn lang03_autoshow_restores_only_current_registered_note_without_writing() {
    let mut h = Harness::new();
    let (a, b) = bind_fixture(&mut h);
    assert!(h.app.open(&mut h.root, a.clone()));
    let original = std::fs::read(&a).unwrap();
    let expected = save_highlight(&mut h);
    let store = AnnotationStore::open(
        &h.app.workspace.data_dir.join("text-annotations.sqlite"),
    ).unwrap();
    assert_eq!(store.load().unwrap().revision, 1);
    assert_eq!(ranges(&h), vec![expected.clone()]);

    assert!(h.app.open(&mut h.root, b));
    assert!(ranges(&h).is_empty(), "another note must not inherit paint");
    assert!(h.app.open(&mut h.root, a.clone()));
    assert_eq!(ranges(&h), vec![expected],
        "a clean note page must automatically restore matching annotations");
    assert_eq!(store.load().unwrap().revision, 1,
        "reopening must not write a sidecar revision");
    assert_eq!(std::fs::read(&a).unwrap(), original);
    eprintln!("AKULA_LANG03_AUTOSHOW_PASS=clean_reopen,correct_note,read_only");
}

#[test]
fn lang03_autoshow_refuses_corrupt_source_digest_and_cold_vault_db_creation() {
    let mut h = Harness::new();
    let (a, b) = bind_fixture(&mut h);
    let db = h.app.workspace.data_dir.join("text-annotations.sqlite");
    assert!(!db.exists());
    assert!(h.app.open(&mut h.root, a.clone()));
    assert!(!db.exists(), "cold note open must not create an annotation database");

    let _ = save_highlight(&mut h);
    let mut store = AnnotationStore::open(&db).unwrap();
    let mut current = store.load().unwrap();
    current.records[0].anchor.document_digest = [0; 32];
    assert_eq!(store.replace(current.revision, &current.records), Ok(2));

    assert!(h.app.open(&mut h.root, b));
    assert!(h.app.open(&mut h.root, a));
    assert!(ranges(&h).is_empty(),
        "changed source digest must HOLD instead of assigning another word");
    assert_eq!(store.load().unwrap().revision, 2);
    eprintln!("AKULA_LANG03_AUTOSHOW_STALE_PASS=cold_no_db,corrupt_digest_hidden");
}
