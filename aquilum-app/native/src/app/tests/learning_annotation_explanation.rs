use super::*;
use super::learning_multi::bind_fixture;
use aquilum_core::text::annotation_store::AnnotationStore;
use crate::ui::editor::{TextAction, TextEditor};
use crate::ui::dialog::DialogAnswer;
use crate::ui::menu::MenuChoice;
use masonry::kurbo::Point;

fn highlight_eat(h: &mut Harness) {
    let ids = h.app.note_ids.as_ref().expect("open note");
    let text = ids.body.get(&h.root).unwrap().inner().text();
    let offset = text.find("eat").expect("fixture word");
    ids.body.edit(&mut h.root, |mut editor| {
        TextEditor::select_byte_range(&mut editor, offset, offset + 3);
    });
    h.app.open_editor_context_menu(&mut h.root, Point::ORIGIN, true, None);
    let layer = match &h.app.popup {
        Some(Popup::EditorMenu { layer, .. }) => *layer,
        _ => panic!("context menu missing"),
    };
    h.act(layer, MenuChoice::Item(1));
    let layer = match &h.app.popup {
        Some(Popup::AnnotationColors { layer, .. }) => *layer,
        _ => panic!("color choices missing"),
    };
    h.act(layer, MenuChoice::Item(0));
}

fn open_note_editor(h: &mut Harness) -> (masonry::core::WidgetId, masonry::core::WidgetId) {
    h.app.open_editor_context_menu(&mut h.root, Point::ORIGIN, true, None);
    let layer = match &h.app.popup {
        Some(Popup::EditorMenu { layer, commands }) => {
            assert!(matches!(commands.get(2), Some(
                super::super::editor_requests::EditorCommand::Explain
            )));
            *layer
        },
        _ => panic!("context menu missing"),
    };
    h.act(layer, MenuChoice::Item(2));
    match &h.app.popup {
        Some(Popup::AnnotationExplanation { dialog, field, .. }) =>
            (field.id(), dialog.layer),
        _ => panic!("explanation editor missing"),
    }
}

#[test]
fn lang03_note_explanation_is_saved_only_in_local_sidecar() {
    let mut h = Harness::new();
    let (a, _) = bind_fixture(&mut h);
    let before = std::fs::read(&a).unwrap();
    assert!(h.app.open(&mut h.root, a.clone()));
    highlight_eat(&mut h);
    let (field, dialog) = open_note_editor(&mut h);
    h.act(field, TextAction::Changed("  Прошедшее время eat  ".into()));
    h.act(dialog, DialogAnswer::Confirm);
    let store = AnnotationStore::open(
        &h.app.workspace.data_dir.join("text-annotations.sqlite"),
    ).unwrap();
    let snapshot = store.load().unwrap();
    assert_eq!(snapshot.records.len(), 1);
    assert_eq!(snapshot.records[0].explanation.as_deref(), Some("Прошедшее время eat"));
    assert_eq!(snapshot.revision, 2);
    assert_eq!(std::fs::read(&a).unwrap(), before);
    eprintln!("AKULA_LANG03_NOTE_PASS=sidecar_only,revision_cas,no_md_mutation");
}

#[test]
fn lang03_note_explanation_rejects_external_change_after_dialog_opens() {
    let mut h = Harness::new();
    let (a, _) = bind_fixture(&mut h);
    assert!(h.app.open(&mut h.root, a.clone()));
    highlight_eat(&mut h);
    let (field, dialog) = open_note_editor(&mut h);
    h.act(field, TextAction::Changed("new note".into()));
    std::fs::write(&a, "| Q | A |\n| --- | --- |\n| ran | изменено |\n").unwrap();
    h.act(dialog, DialogAnswer::Confirm);
    assert!(matches!(h.app.popup, Some(Popup::StudyPreview { .. })),
        "changed file must refuse metadata write");
    let store = AnnotationStore::open(
        &h.app.workspace.data_dir.join("text-annotations.sqlite"),
    ).unwrap();
    let snapshot = store.load().unwrap();
    assert_eq!(snapshot.revision, 1);
    assert!(snapshot.records[0].explanation.is_none());
    eprintln!("AKULA_LANG03_NOTE_STALE_PASS=backing_file_changed,no_write");
}

#[test]
fn lang03_note_explanation_escape_closes_without_changes() {
    let mut h = Harness::new();
    let (a, _) = bind_fixture(&mut h);
    assert!(h.app.open(&mut h.root, a));
    highlight_eat(&mut h);
    let (_, dialog) = open_note_editor(&mut h);
    h.act(dialog, DialogAnswer::Closed);
    assert!(h.app.popup.is_none());
    let store = AnnotationStore::open(
        &h.app.workspace.data_dir.join("text-annotations.sqlite"),
    ).unwrap();
    assert_eq!(store.load().unwrap().revision, 1);
}
