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
