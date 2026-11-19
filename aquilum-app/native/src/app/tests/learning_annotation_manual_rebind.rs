//! LANG-03p: first-party UI action with real SQLite and selection events.
use super::*;
use super::learning_multi::bind_fixture;
use aquilum_core::text::annotation_backup::{export_document, import_document};
use aquilum_core::text::annotation_store::{
    AnnotationRecord, AnnotationStore, BindingStatus, PaletteColor, Snapshot,
};
use aquilum_core::text::annotations::capture;
use crate::ui::editor::TextEditor;
use crate::ui::menu::MenuChoice;
use masonry::kurbo::Point;

fn store(h: &Harness) -> AnnotationStore {
    AnnotationStore::open(
        &h.app.workspace.data_dir.join("text-annotations.sqlite"),
    ).unwrap()
}

fn setup(h: &mut Harness) -> (std::path::PathBuf, usize) {
    let (a, _) = bind_fixture(h);
    assert!(h.app.open(&mut h.root, a.clone()));
    let current = h.app.authorized_annotation_editor(&h.root).unwrap();
    let start = current.body.find("eat").expect("fixture selection");
    let record = AnnotationRecord {
        anchor: capture(&current.document, "restored-a", &current.body,
            start..start + 3).unwrap(),
        color: PaletteColor::Blue,
        category: Some("verbs".into()),
        explanation: Some("from backed-up note".into()),
        reference: None,
        status: BindingStatus::Current,
    };
    let backup = export_document(&Snapshot {
        revision: 0, records: vec![record],
    }, &current.document).unwrap();
    let mut db = store(h);
    let result = import_document(&mut db, 0, &current.document, &backup).unwrap();
    assert_eq!((result.imported, result.held), (1, 1));
    assert_eq!(db.load().unwrap().records[0].status, BindingStatus::NeedsRebind);
    drop(db);
    let ids = h.app.note_ids.as_ref().unwrap();
    ids.body.edit(&mut h.root, |mut editor| {
        TextEditor::select_byte_range(&mut editor, start, start + 3);
    });
    (a, start)
}

fn choose_rebind(h: &mut Harness) {
    h.app.open_editor_context_menu(&mut h.root, Point::ORIGIN, true, None);
    let (layer, index) = match &h.app.popup {
        Some(Popup::EditorMenu { layer, commands }) => {
            assert!(matches!(commands.get(1),
                Some(super::super::editor_requests::EditorCommand::Highlight)),
                "old highlight menu position must not change");
            let index = commands.iter().position(|c| matches!(c,
                super::super::editor_requests::EditorCommand::RebindHeld))
                .expect("manual rebind entry must exist");
            assert_eq!(index, commands.len() - 1,
                "new command must append without shifting old positions");
            (*layer, index)
        }
        _ => panic!("editor context menu must exist"),
    };
    h.act(layer, MenuChoice::Item(index));
}

#[test]
fn lang03_native_imported_hold_rebinds_explicit_selection_without_touching_markdown() {
    let mut h = Harness::new();
    let (a, at) = setup(&mut h);
    let source = std::fs::read(&a).unwrap();
    let old = store(&h).load().unwrap();
    choose_rebind(&mut h);
    let next = store(&h).load().unwrap();
    assert_eq!(next.revision, old.revision + 1);
    assert_eq!(next.records.len(), 1);
    assert_eq!(next.records[0].status, BindingStatus::Current);
    assert_eq!((next.records[0].anchor.byte_start, next.records[0].anchor.byte_end),
        (at, at + 3));
    assert_eq!(next.records[0].explanation.as_deref(),
        Some("from backed-up note"));
    let ranges = h.app.note_ids.as_ref().unwrap().body.get(&h.root)
        .unwrap().inner().annotation_ranges_for_test();
    assert_eq!(ranges, vec![at..at + 3]);
    assert_eq!(std::fs::read(a).unwrap(), source,
        "manual rebind must not alter source Markdown");
    eprintln!("AKULA_LANG03_MANUAL_REBIND_PASS=import_hold,explicit_selection,sqlite_cas,no_source_write");
}

#[test]
fn lang03_native_ambiguous_held_records_are_not_guessed_or_overwritten() {
    let mut h = Harness::new();
    let (a, _) = setup(&mut h);
    let mut db = store(&h);
    let snapshot = db.load().unwrap();
    let mut second = snapshot.records[0].clone();
    second.anchor.annotation_id = "restored-b".into();
    let mut rows = snapshot.records.clone();
    rows.push(second);
    assert_eq!(db.replace(snapshot.revision, &rows), Ok(snapshot.revision + 1));
    drop(db);
    let original = std::fs::read(&a).unwrap();
    let before = store(&h).load().unwrap();
    choose_rebind(&mut h);
    let after = store(&h).load().unwrap();
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.records, before.records);
    assert_eq!(std::fs::read(a).unwrap(), original);
    eprintln!("AKULA_LANG03_MANUAL_REBIND_AMBIGUOUS=held_no_guess,no_write");
}

#[test]
fn lang03_native_external_rewrite_refuses_stale_selected_source() {
    let mut h = Harness::new();
    let (a, _) = setup(&mut h);
    let before = store(&h).load().unwrap();
    let edited = std::fs::read_to_string(&a).unwrap().replacen("eat", "ate", 1);
    std::fs::write(&a, &edited).unwrap();
    choose_rebind(&mut h);
    let after = store(&h).load().unwrap();
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.records, before.records);
    assert_eq!(std::fs::read_to_string(a).unwrap(), edited);
    eprintln!("AKULA_LANG03_MANUAL_REBIND_STALE=backing_file_changed,no_sidecar_write");
}
