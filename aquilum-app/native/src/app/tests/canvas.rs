//! End-to-end native App route for the real JSON Canvas tab, not a standalone widget.
use super::*;
use std::fs;

fn shot(h: &mut Harness) -> image::RgbaImage {
    let png = h.frame(W as u16, H as u16, 1.0).png();
    image::load_from_memory(&png).expect("native UI frame PNG").to_rgba8()
}

#[test]
fn canvas_tree_to_native_tab_switch_and_reopen_preserves_bytes() {
    let mut h = Harness::new();
    let vault = h.vault.path().to_path_buf();
    let board = vault.join("board.canvas");
    let empty = vault.join("blank.canvas");
    let original = r#"{
  "nodes": [
    {"id":"g","type":"group","x":0,"y":0,"width":430,"height":270,"label":"Project"},
    {"id":"a","type":"text","x":20,"y":30,"width":190,"height":120,"text":"First card"},
    {"id":"b","type":"text","x":250,"y":115,"width":160,"height":95,"text":"Second card"}
  ],
  "edges": [{"id":"e","fromNode":"a","toNode":"b","fromSide":"right","toSide":"left","toEnd":"arrow"}],
  "thirdParty":{"keep":true}
}"#;
    fs::write(&board, original.as_bytes()).unwrap();
    fs::write(&empty, br#"{"nodes":[],"edges":[]}"#).unwrap();
    h.app.tree.paths_changed(&mut h.root, &[board.clone(), empty.clone()]);
    h.settle();

    // Real user path: click .canvas in the file tree, not a direct CanvasPage
    // constructor or a raw standalone JSON renderer.
    let id = h.app.tree.row_id("board.canvas").expect("Canvas must be visible in the file tree");
    let at = h.root.get_widget(id).expect("Canvas file row").ctx().bounding_box().center();
    h.click(at.x, at.y);
    assert_eq!(h.app.tabs.active().path.as_deref(), Some(board.as_path()));
    assert!(h.app.note.is_none(), "Canvas must not be opened in the Markdown editor");
    assert!(h.app.open_error_ids.is_none(), "valid Canvas must not show an error");
    let populated = shot(&mut h);

    assert!(h.app.open(&mut h.root, empty.clone()));
    assert!(h.app.open_error_ids.is_none());
    let blank = shot(&mut h);

    // Compare the actual central board area, not the tab title, file tree or
    // the status counter. This detects accidentally opening the text preview
    // or showing an empty native widget instead of positioned Canvas cards.
    let different = (180..520).flat_map(|y| (360..780).map(move |x| (x, y)))
        .filter(|&(x, y)| populated.get_pixel(x, y) != blank.get_pixel(x, y))
        .count();
    assert!(different > 1_000, "Canvas area lacks visible cards: {different} pixels");

    assert!(h.app.open(&mut h.root, vault.join("a.md")));
    assert!(h.app.open(&mut h.root, board.clone()));
    assert!(h.app.open_error_ids.is_none(), "reopened Canvas shows an error");
    let reopened = shot(&mut h);
    assert_eq!(fs::read(&board).unwrap(), original.as_bytes(), "open/switch/reopen changed source bytes");
    // Both snapshots were built by the full native App for the same board.
    assert_eq!(populated.as_raw(), reopened.as_raw(), "Canvas changed after tab roundtrip");
}

#[test]
fn corrupt_canvas_in_real_tab_is_error_page_and_keeps_original_bytes() {
    let mut h = Harness::new();
    let vault = h.vault.path().to_path_buf();
    let path = vault.join("broken.canvas");
    let broken = br#"{"nodes":{"not":"an array"},"edges":[]}"#;
    fs::write(&path, broken).unwrap();
    assert!(h.app.open(&mut h.root, path.clone()));
    assert!(h.app.open_error_ids.is_some(), "invalid Canvas must render an error page");
    let _ = h.root.redraw();
    assert_eq!(fs::read(&path).unwrap(), broken, "error view must never rewrite the file");
}


/// Use the same primary pointer gestures and toolbar clicks as the installed UI.
fn canvas_local_point(h: &Harness, point: impl FnOnce(&crate::ui::canvas::CanvasPage) -> masonry::kurbo::Point) -> (f64, f64) {
    let id = h.app.canvas_view_id().expect("native Canvas widget exists");
    let widget = h.root.get_widget(id).expect("Canvas widget in native root");
    let top_left = widget.ctx().bounding_box().origin();
    let local = point(widget.downcast::<crate::ui::canvas::CanvasPage>().unwrap().inner());
    (top_left.x + local.x, top_left.y + local.y)
}

/// Deliver a real Down -> Move -> Up pointer sequence through RenderRoot.
fn canvas_drag(h: &mut Harness, from: (f64, f64), to: (f64, f64)) {
    h.settle();
    let mut state = PointerState {
        position: PhysicalPosition::new(from.0, from.1),
        count: 1,
        scale_factor: 1.0,
        ..PointerState::default()
    };
    h.root.handle_pointer_event(PointerEvent::Move(PointerUpdate {
        pointer: MOUSE, current: state.clone(), coalesced: vec![], predicted: vec![],
    }));
    state.buttons.insert(PointerButton::Primary);
    h.root.handle_pointer_event(PointerEvent::Down(PointerButtonEvent {
        pointer: MOUSE, button: Some(PointerButton::Primary), state: state.clone(),
    }));
    state.position = PhysicalPosition::new(to.0, to.1);
    h.root.handle_pointer_event(PointerEvent::Move(PointerUpdate {
        pointer: MOUSE, current: state.clone(), coalesced: vec![], predicted: vec![],
    }));
    state.buttons.remove(PointerButton::Primary);
    h.root.handle_pointer_event(PointerEvent::Up(PointerButtonEvent {
        pointer: MOUSE, button: Some(PointerButton::Primary), state,
    }));
    h.deliver();
}

fn canvas_control(h: &mut Harness, index: usize) {
    let at = canvas_local_point(h, |view| view.toolbar_center(index));
    h.click(at.0, at.1);
}

fn canvas_card(h: &Harness, id: &str) -> (f64, f64) {
    canvas_local_point(h, |view| view.card_center(id).expect("visible text card"))
}

fn canvas_disk(path: &std::path::Path) -> serde_json::Value {
    aquilum_core::files::structured::StructuredDocument::open(path).unwrap().canvas().unwrap()
}

#[test]
fn canvas_real_pointer_move_undo_redo_and_create_auto_save_via_file_gate() {
    let mut h = Harness::new();
    let board = h.vault.path().join("editing.canvas");
    let original = r#"{"nodes":[{"id":"a","type":"text","x":20,"y":30,"width":190,"height":100,"text":"move me","vendor":{"keep":true}}],"edges":[],"plugin":{"root":"keep"}}"#;
    fs::write(&board, original).unwrap();
    assert!(h.app.open(&mut h.root, board.clone()));
    h.settle();
    assert!(!h.app.canvas_dirty_active());

    let before = canvas_disk(&board);
    let at = canvas_card(&h, "a");
    canvas_drag(&mut h, at, (at.0 + 78.0, at.1 + 42.0));
    assert!(!h.app.canvas_dirty_active(), "pointer release should File Gate save");
    let moved = canvas_disk(&board);
    assert_ne!(moved["nodes"][0]["x"], before["nodes"][0]["x"]);
    assert_ne!(moved["nodes"][0]["y"], before["nodes"][0]["y"]);
    assert_eq!(moved["plugin"]["root"], "keep");
    assert_eq!(moved["nodes"][0]["vendor"]["keep"], true);

    canvas_control(&mut h, 1); // Undo
    assert_eq!(canvas_disk(&board)["nodes"][0]["x"], before["nodes"][0]["x"]);
    canvas_control(&mut h, 2); // Redo
    assert_eq!(canvas_disk(&board)["nodes"][0]["x"], moved["nodes"][0]["x"]);

    canvas_control(&mut h, 0); // + Text
    assert_eq!(canvas_disk(&board)["nodes"].as_array().unwrap().len(), 2);
    assert!(!h.app.canvas_dirty_active());

    assert!(h.app.open(&mut h.root, h.vault.path().join("a.md")));
    assert!(h.app.open(&mut h.root, board.clone()));
    assert!(h.app.open_error_ids.is_none());
    assert_eq!(canvas_disk(&board)["nodes"].as_array().unwrap().len(), 2);
}

#[test]
fn canvas_external_conflict_keeps_unsaved_edit_and_blocks_tab_close() {
    let mut h = Harness::new();
    let board = h.vault.path().join("conflict.canvas");
    fs::write(&board, r#"{"nodes":[{"id":"a","type":"text","x":0,"y":0,"width":200,"height":100,"text":"draft"}]}"#).unwrap();
    assert!(h.app.open(&mut h.root, board.clone()));
    h.settle();

    let external = r#"{"nodes":[{"id":"a","type":"text","x":0,"y":0,"width":200,"height":100,"text":"external"}],"external":true}"#;
    fs::write(&board, external).unwrap();
    let at = canvas_card(&h, "a");
    canvas_drag(&mut h, at, (at.0 + 70.0, at.1 + 40.0));

    assert!(h.app.canvas_dirty_active(), "File Gate conflict must keep local work");
    assert_eq!(fs::read_to_string(&board).unwrap(), external, "never overwrite external changes");
    let active = h.app.tabs.active_id();
    h.app.close_active_tab(&mut h.root);
    assert_eq!(h.app.tabs.active_id(), active, "unsaved conflicting Canvas cannot silently close");

    // Undo discards the unsaved move (the disk remains the external source).
    canvas_control(&mut h, 1);
    assert!(!h.app.canvas_dirty_active());
    assert_eq!(fs::read_to_string(&board).unwrap(), external);
}

#[test]
fn canvas_cannot_open_outside_vault_or_follow_symlink_outside_it() {
    let mut h = Harness::new();
    let outside = tempfile::tempdir().unwrap();
    let path = outside.path().join("private.canvas");
    let raw = br#"{"nodes":[],"private":true}"#;
    fs::write(&path, raw).unwrap();

    assert!(h.app.open(&mut h.root, path.clone()));
    assert!(h.app.open_error_ids.is_some());
    assert!(h.app.open_error.as_ref().unwrap().1.contains("outside"));
    assert_eq!(fs::read(&path).unwrap(), raw);

    #[cfg(unix)]
    {
        let symlink = h.vault.path().join("external.canvas");
        std::os::unix::fs::symlink(&path, &symlink).unwrap();
        assert!(h.app.open(&mut h.root, symlink));
        assert!(h.app.open_error_ids.is_some());
        assert!(h.app.open_error.as_ref().unwrap().1.contains("outside"));
    }
}

#[test]
fn canvas_workspace_switch_keeps_dirty_file_and_clears_clean_sessions() {
    let mut h = Harness::new();
    let old_root = h.vault.path().to_path_buf();
    let other = tempfile::tempdir().unwrap();
    fs::write(other.path().join("a.md"), "other vault").unwrap();
    let board = old_root.join("switch.canvas");
    fs::write(&board, r#"{"nodes":[{"id":"a","type":"text","x":0,"y":0,"width":200,"height":100,"text":"draft"}]}"#).unwrap();
    assert!(h.app.open(&mut h.root, board.clone()));
    h.settle();

    let external = r#"{"nodes":[{"id":"a","type":"text","x":0,"y":0,"width":200,"height":100,"text":"external"}],"external":true}"#;
    fs::write(&board, external).unwrap();
    let at = canvas_card(&h, "a");
    canvas_drag(&mut h, at, (at.0 + 76.0, at.1 + 41.0));
    assert!(h.app.canvas_dirty_active());

    h.app.switch_workspace(&mut h.root, other.path().to_path_buf());
    assert_eq!(h.app.tree.root(), Some(old_root.as_path()), "dirty Canvas must block vault switch");
    assert!(h.app.canvas_dirty_active(), "pending edits must remain recoverable");
    assert_eq!(fs::read_to_string(&board).unwrap(), external);

    canvas_control(&mut h, 1); // Undo the conflicted move
    assert!(!h.app.canvas_dirty_active());
    h.app.switch_workspace(&mut h.root, other.path().to_path_buf());
    assert_eq!(h.app.tree.root(), Some(other.path()));
    assert!(h.app.canvas_view_id().is_none(), "previous vault must not retain Canvas handles");
    assert_eq!(fs::read_to_string(&board).unwrap(), external);
}

#[test]
fn canvas_click_selection_delete_undo_redo_keep_other_links_and_vendor_fields() {
    let mut h = Harness::new();
    let board = h.vault.path().join("delete.canvas");
    let raw = r#"{"nodes":[{"id":"a","type":"text","x":20,"y":25,"width":160,"height":90,"text":"remove me","vendor":{"note":true}},{"id":"b","type":"text","x":280,"y":170,"width":160,"height":90,"text":"keep"},{"id":"group","type":"group","x":0,"y":0,"width":490,"height":300,"label":"group"}],"edges":[{"id":"lost","fromNode":"a","toNode":"b"},{"id":"kept","fromNode":"b","toNode":"group"}],"plugin":{"opaque":"keep"}}"#;
    fs::write(&board, raw).unwrap();
    assert!(h.app.open(&mut h.root, board.clone()));
    h.settle();
    let original = fs::read(&board).unwrap();
    canvas_control(&mut h, 4);
    assert_eq!(fs::read(&board).unwrap(), original, "delete with no selection must not write");
    let at = canvas_card(&h, "a");
    h.click(at.0, at.1);
    canvas_control(&mut h, 4);
    let deleted = canvas_disk(&board);
    let nodes = deleted["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 2);
    assert!(nodes.iter().all(|node| node["id"].as_str() != Some("a")));
    assert_eq!(deleted["edges"].as_array().unwrap().len(), 1);
    assert_eq!(deleted["edges"][0]["id"], "kept");
    assert_eq!(deleted["plugin"]["opaque"], "keep");
    assert!(!h.app.canvas_dirty_active(), "File Gate must persist the deletion");
    canvas_control(&mut h, 1);
    let original_json: serde_json::Value = serde_json::from_str(raw).unwrap();
    assert_eq!(canvas_disk(&board), original_json, "one Undo restores node and dependent edge");
    canvas_control(&mut h, 2);
    assert_eq!(canvas_disk(&board), deleted, "Redo removes the same node and edge");
    assert!(h.app.open(&mut h.root, h.vault.path().join("a.md")));
    assert!(h.app.open(&mut h.root, board.clone()));
    assert_eq!(canvas_disk(&board), deleted);
}

#[test]
fn canvas_delete_on_external_conflict_preserves_file_and_local_undo_history() {
    let mut h = Harness::new();
    let board = h.vault.path().join("delete-conflict.canvas");
    fs::write(&board, r#"{"nodes":[{"id":"a","type":"text","x":0,"y":0,"width":200,"height":100,"text":"original"}],"edges":[]}"#).unwrap();
    assert!(h.app.open(&mut h.root, board.clone()));
    h.settle();
    let at = canvas_card(&h, "a");
    h.click(at.0, at.1);
    let external = r#"{"nodes":[{"id":"a","type":"text","x":0,"y":0,"width":200,"height":100,"text":"external"}],"vendor":{"external":true}}"#;
    fs::write(&board, external).unwrap();
    canvas_control(&mut h, 4);
    assert!(h.app.canvas_dirty_active());
    assert_eq!(fs::read_to_string(&board).unwrap(), external);
    let active = h.app.tabs.active_id();
    h.app.close_active_tab(&mut h.root);
    assert_eq!(h.app.tabs.active_id(), active, "unsaved conflict must prevent closing");
    canvas_control(&mut h, 1);
    assert!(!h.app.canvas_dirty_active());
    assert_eq!(fs::read_to_string(&board).unwrap(), external);
}
