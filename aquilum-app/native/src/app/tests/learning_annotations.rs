use super::*;
use super::learning_multi::bind_fixture;
use aquilum_core::text::annotation_store::{AnnotationStore, PaletteColor};
use crate::ui::editor::TextEditor;
use crate::ui::menu::MenuChoice;
use masonry::kurbo::Point;

fn start_palette(h: &mut Harness) {
    let ids = h.app.note_ids.as_ref().unwrap();
    let text = ids.body.get(&h.root).unwrap().inner().text();
    let at = text.find("eat").expect("fixture contains eat");
    ids.body.edit(&mut h.root, |mut editor| {
        TextEditor::select_byte_range(&mut editor, at, at + 3);
    });
    h.app.open_editor_context_menu(&mut h.root, Point::ORIGIN, true, None);
    let layer = match &h.app.popup {
        Some(Popup::EditorMenu { layer, commands }) => {
            assert!(matches!(commands.get(1), Some(super::super::editor_requests::EditorCommand::Highlight)));
            *layer
        }
        _ => panic!("editor context menu not opened"),
    };
    h.act(layer, MenuChoice::Item(1));
    assert!(matches!(h.app.popup, Some(Popup::AnnotationColors { .. })),
        "color palette should open for selected text");
}

#[test]
fn lang03_native_palette_saves_color_and_preserves_markdown_bytes() {
    let mut h = Harness::new();
    let (a, _) = bind_fixture(&mut h);
    let original = std::fs::read(&a).unwrap();
    assert!(h.app.open(&mut h.root, a.clone()));
    start_palette(&mut h);
    let layer = match &h.app.popup {
        Some(Popup::AnnotationColors { layer, .. }) => *layer,
        _ => unreachable!(),
    };
    h.act(layer, MenuChoice::Item(3)); // Blue
    let store = AnnotationStore::open(
        &h.app.workspace.data_dir.join("text-annotations.sqlite"),
    ).unwrap();
    let snapshot = store.load().unwrap();
    assert_eq!(snapshot.records.len(), 1);
    assert_eq!(snapshot.records[0].color, PaletteColor::Blue);
    assert_eq!(snapshot.revision, 1);
    assert_eq!(std::fs::read(&a).unwrap(), original, "annotation must not edit Markdown");

    h.app.hide_annotation_overlay(&mut h.root);
    h.app.refresh_annotation_overlay(&mut h.root);
    let again = store.load().unwrap();
    assert_eq!(again.revision, 1, "show and hide do not persist grades or text");
    eprintln!("AKULA_LANG03_COLOR_PASS=metadata_only,show_hide,no_markdown_write");
}

#[test]
fn lang03_native_palette_external_edit_aborts_before_saving_a_stale_span() {
    let mut h = Harness::new();
    let (a, _) = bind_fixture(&mut h);
    assert!(h.app.open(&mut h.root, a.clone()));
    start_palette(&mut h);
    let layer = match &h.app.popup {
        Some(Popup::AnnotationColors { layer, .. }) => *layer,
        _ => unreachable!(),
    };
    std::fs::write(&a, "| Q | A |\n| --- | --- |\n| ran | бежал |\n").unwrap();
    h.act(layer, MenuChoice::Item(0));
    assert!(matches!(h.app.popup, Some(Popup::StudyPreview { .. })),
        "changed external file must refuse annotation");
    let store = AnnotationStore::open(
        &h.app.workspace.data_dir.join("text-annotations.sqlite"),
    ).unwrap();
    assert!(store.load().unwrap().records.is_empty());
    eprintln!("AKULA_LANG03_STALE_PASS=backing_hash_changed,no_metadata_write");
}
