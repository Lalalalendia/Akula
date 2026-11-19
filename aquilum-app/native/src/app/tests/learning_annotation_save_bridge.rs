use super::*;
use super::learning_multi::bind_fixture;
use aquilum_core::text::annotation_store::{AnnotationStore, BindingStatus};
use crate::ui::editor::TextEditor;
use crate::ui::menu::MenuChoice;
use masonry::kurbo::Point;

fn open_highlighted(h: &mut Harness) -> std::path::PathBuf {
    let (a, _) = bind_fixture(h);
    assert!(h.app.open(&mut h.root, a.clone()));
    let ids = h.app.note_ids.as_ref().unwrap();
    let current = ids.body.get(&h.root).unwrap().inner().text();
    let at = current.find("eat").unwrap();
    ids.body.edit(&mut h.root, |mut editor| {
        TextEditor::select_byte_range(&mut editor, at, at + 3);
    });
    h.app.open_editor_context_menu(&mut h.root, Point::ORIGIN, true, None);
    let menu = match &h.app.popup {
        Some(Popup::EditorMenu { layer, .. }) => *layer,
        _ => panic!("missing editor context menu"),
    };
    h.act(menu, MenuChoice::Item(1));
    let palette = match &h.app.popup {
        Some(Popup::AnnotationColors { layer, .. }) => *layer,
        _ => panic!("missing colors"),
    };
    h.act(palette, MenuChoice::Item(1));
    a
}

fn sidecar(h: &Harness) -> AnnotationStore {
    AnnotationStore::open(
        &h.app.workspace.data_dir.join("text-annotations.sqlite"),
    ).unwrap()
}

fn insert(h: &mut Harness, start: usize, end: usize, new: &str) {
    let ids = h.app.note_ids.as_ref().unwrap();
    ids.body.edit(&mut h.root, |mut editor| {
        TextEditor::select_byte_range(&mut editor, start, end);
        TextEditor::insert_text(&mut editor, new);
    });
    h.deliver();
}

fn finish_save(h: &mut Harness) {
    let core = std::sync::Arc::clone(&h.app.workspace.core);
    let content = h.app.text.clone();
    h.app.note.as_mut().unwrap().flush(&core, &content);
    h.app.finish_annotation_rebind_after_save(&mut h.root);
}

#[test]
fn lang03_real_save_moves_highlight_only_after_disk_ack_and_cas() {
    let mut h = Harness::new();
    let a = open_highlighted(&mut h);
    let old = sidecar(&h).load().unwrap();
    assert_eq!(old.revision, 1);
    let before = old.records[0].anchor.byte_start;

    insert(&mut h, 0, 0, "вступление ");
    assert!(h.app.pending_annotation_rebind.is_some());
    assert_eq!(sidecar(&h).load().unwrap().revision, 1,
        "dirty Markdown may not move metadata before ACK");

    finish_save(&mut h);
    let saved = sidecar(&h).load().unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(saved.records[0].status, BindingStatus::Current);
    assert_eq!(saved.records[0].anchor.byte_start, before + "вступление ".len());
    assert!(std::fs::read_to_string(&a).unwrap().starts_with("вступление "));
    assert!(h.app.pending_annotation_rebind.is_none());
    eprintln!("AKULA_LANG03_SAVE_BRIDGE_PASS=after_disk_ack,revision_cas");
}

#[test]
fn lang03_real_save_holds_edited_selection_instead_of_guessing_duplicate() {
    let mut h = Harness::new();
    let a = open_highlighted(&mut h);
    let old = sidecar(&h).load().unwrap();
    let at = old.records[0].anchor.byte_start;
    insert(&mut h, at, at + 3, "ate");
    assert_eq!(sidecar(&h).load().unwrap().revision, 1);
    finish_save(&mut h);
    let saved = sidecar(&h).load().unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(saved.records[0].status, BindingStatus::NeedsRebind);
    assert_eq!(saved.records[0].anchor.byte_start, at);
    assert!(std::fs::read_to_string(&a).unwrap().contains("ate"));
    eprintln!("AKULA_LANG03_SAVE_BRIDGE_HOLD=selection_edited");
}

#[test]
fn lang03_concurrent_sidecar_change_cannot_be_overwritten_after_save() {
    let mut h = Harness::new();
    let _ = open_highlighted(&mut h);
    insert(&mut h, 0, 0, "intro ");
    assert!(h.app.pending_annotation_rebind.is_some());
    let mut store = sidecar(&h);
    let mut snap = store.load().unwrap();
    snap.records[0].explanation = Some("changed externally".into());
    assert_eq!(store.replace(snap.revision, &snap.records), Ok(2));

    finish_save(&mut h);
    let saved = store.load().unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(saved.records[0].explanation.as_deref(), Some("changed externally"));
    assert_eq!(saved.records[0].anchor, snap.records[0].anchor);
    assert!(h.app.pending_annotation_rebind.is_none());
    eprintln!("AKULA_LANG03_SAVE_BRIDGE_STALE=concurrent_cas_refused");
}

#[test]
fn lang03_switching_note_discards_uncommitted_coordinate_history() {
    let mut h = Harness::new();
    let a = open_highlighted(&mut h);
    let b = h.vault.path().join("b.md");
    insert(&mut h, 0, 0, "intro ");
    assert!(h.app.pending_annotation_rebind.is_some());
    assert!(h.app.open(&mut h.root, b));
    assert!(h.app.pending_annotation_rebind.is_none());
    assert_eq!(sidecar(&h).load().unwrap().revision, 1);
    assert!(a.exists());
}
