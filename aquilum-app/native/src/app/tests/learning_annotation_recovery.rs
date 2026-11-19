//! LANG-03n: evidence for persisted annotation behavior across a native
//! restart and an independently changed backing Markdown file.
use super::*;
use super::learning_multi::bind_fixture;
use aquilum_core::text::annotation_store::{AnnotationStore, BindingStatus};
use crate::ui::editor::TextEditor;
use crate::ui::menu::MenuChoice;
use masonry::kurbo::Point;

fn sidecar(h: &Harness) -> AnnotationStore {
    AnnotationStore::open(&h.app.workspace.data_dir.join("text-annotations.sqlite")).unwrap()
}

fn spans(h: &Harness) -> Vec<std::ops::Range<usize>> {
    h.app.note_ids.as_ref().unwrap().body.get(&h.root)
        .unwrap().inner().annotation_ranges_for_test()
}

fn save_highlight(h: &mut Harness) -> std::path::PathBuf {
    let (a, _) = bind_fixture(h);
    assert!(h.app.open(&mut h.root, a.clone()));
    let ids = h.app.note_ids.as_ref().unwrap();
    let text = ids.body.get(&h.root).unwrap().inner().text();
    let at = text.find("eat").expect("fixture word");
    ids.body.edit(&mut h.root, |mut editor| {
        TextEditor::select_byte_range(&mut editor, at, at + 3);
    });
    h.app.open_editor_context_menu(&mut h.root, Point::ORIGIN, true, None);
    let layer = match &h.app.popup {
        Some(Popup::EditorMenu { layer, .. }) => *layer,
        _ => panic!("editor context menu missing"),
    };
    h.act(layer, MenuChoice::Item(1));
    let layer = match &h.app.popup {
        Some(Popup::AnnotationColors { layer, .. }) => *layer,
        _ => panic!("annotation palette missing"),
    };
    h.act(layer, MenuChoice::Item(0));
    assert_eq!(sidecar(h).load().unwrap().revision, 1);
    a
}

#[test]
fn lang03_save_then_restart_restores_exact_moved_span_without_rewriting_markdown() {
    let mut h = Harness::new();
    let a = save_highlight(&mut h);
    let original = sidecar(&h).load().unwrap().records[0].anchor.byte_start;
    let ids = h.app.note_ids.as_ref().unwrap();
    ids.body.edit(&mut h.root, |mut editor| {
        TextEditor::select_byte_range(&mut editor, 0, 0);
        TextEditor::insert_text(&mut editor, "Начало ");
    });
    h.deliver();
    assert_eq!(sidecar(&h).load().unwrap().revision, 1,
        "dirty editor must not move persisted annotations");

    let core = std::sync::Arc::clone(&h.app.workspace.core);
    let text = h.app.text.clone();
    h.app.note.as_mut().unwrap().flush(&core, &text);
    h.app.finish_annotation_rebind_after_save(&mut h.root);
    let saved = sidecar(&h).load().unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(saved.records[0].status, BindingStatus::Current);
    let moved = saved.records[0].anchor.byte_start..saved.records[0].anchor.byte_end;
    assert_eq!(moved.start, original + "Начало ".len());
    assert_eq!(spans(&h), vec![moved.clone()]);
    let persisted_markdown = std::fs::read(&a).unwrap();

    let mut restarted = h.restarted(1.0);
    assert!(restarted.app.open(&mut restarted.root, a.clone()));
    let state = sidecar(&restarted).load().unwrap();
    assert_eq!(state.revision, 2, "read-only startup must not write the sidecar");
    assert_eq!(state.records[0].anchor.byte_start..state.records[0].anchor.byte_end, moved);
    assert_eq!(spans(&restarted), vec![moved]);
    assert_eq!(std::fs::read(&a).unwrap(), persisted_markdown);
    eprintln!("AKULA_LANG03_RESTART_PASS=exact_span,read_only_reopen");
}

#[test]
fn lang03_external_watcher_change_does_not_retarget_annotation_or_write_sidecar() {
    let mut h = Harness::new();
    h.watch_vault();
    let a = save_highlight(&mut h);
    let before = sidecar(&h).load().unwrap();
    assert_eq!(before.records.len(), 1);
    assert_eq!(spans(&h).len(), 1);
    let source = std::fs::read_to_string(&a).unwrap();
    let changed = source.replacen("eat", "ate", 1);
    assert_ne!(source, changed);
    std::fs::write(&a, &changed).unwrap();
    h.app.workspace.core.emit(aquilum_core::CoreEvent::WorkspaceRescanned);
    assert!(wait_until(250, 20, || {
        h.app.on_core_events(&mut h.root);
        h.body_text() == changed
    }), "watcher must adopt the external edit");
    assert!(spans(&h).is_empty(),
        "external text change must not repaint an unverified annotation");
    let after = sidecar(&h).load().unwrap();
    assert_eq!(after.revision, before.revision,
        "watcher reload may not change persisted annotation revision");
    assert_eq!(after.records, before.records,
        "duplicate phrases or changed text cannot relocate stored anchors");
    eprintln!("AKULA_LANG03_WATCHER_PASS=stale_hidden,no_retarget,no_write");
}

#[test]
fn lang03_native_undo_redo_replays_verified_edits_before_disk_ack() {
    let mut h = Harness::new();
    let a = save_highlight(&mut h);
    let before = h.body_text();
    let original = sidecar(&h).load().unwrap().records[0].anchor.byte_start;
    let body = h.app.note_ids.as_ref().unwrap().body;
    body.edit(&mut h.root, |mut editor| {
        TextEditor::select_byte_range(&mut editor, 0, 0);
        TextEditor::insert_text(&mut editor, "prefix ");
    });
    h.deliver();
    assert_eq!(h.body_text(), format!("prefix {before}"));
    h.root.focus_on(Some(body.id()));
    h.key(Key::Character("z".into()), Modifiers::CONTROL);
    assert_eq!(h.body_text(), before, "Ctrl+Z must undo the real editor edit");
    assert_eq!(sidecar(&h).load().unwrap().revision, 1);
    h.key(Key::Character("y".into()), Modifiers::CONTROL);
    assert_eq!(h.body_text(), format!("prefix {before}"),
        "Ctrl+Y must redo the exact previously verified editor change");
    assert_eq!(sidecar(&h).load().unwrap().revision, 1,
        "neither shortcut may persist coordinates before the native saver ACK");

    let core = std::sync::Arc::clone(&h.app.workspace.core);
    let current = h.app.text.clone();
    h.app.note.as_mut().unwrap().flush(&core, &current);
    h.app.finish_annotation_rebind_after_save(&mut h.root);
    let saved = sidecar(&h).load().unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(saved.records[0].status, BindingStatus::Current);
    let moved = saved.records[0].anchor.byte_start..saved.records[0].anchor.byte_end;
    assert_eq!(moved.start, original + "prefix ".len());
    assert_eq!(spans(&h), vec![moved]);
    assert_eq!(std::fs::read_to_string(&a).unwrap(), h.body_text());
    eprintln!("AKULA_LANG03_UNDO_REDO_PASS=real_shortcuts,after_save_only");
}
