//! C5 JSON Canvas 1.0 file-format interoperability contracts.
//! Synthetic fixture following https://github.com/obsidianmd/jsoncanvas/blob/main/spec/1.0.md
//! These tests do NOT claim a GUI round trip through the Obsidian application.
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use aquilum_core::CoreEvent;
use aquilum_core::app_core::Core;
use aquilum_core::files::error::FileCommandError;
use aquilum_core::files::structured::{StructuredDocument, StructuredError};
use aquilum_core::files::structured::session::{CanvasSession, CanvasSessionError};
use serde_json::{Value, json};

const SPEC_CANVAS: &str = r#"{
  "nodes": [
    {"id":"group","type":"group","x":-20,"y":-30,"width":700,"height":450,
      "label":"Research","background":"Pictures/map.svg","backgroundStyle":"ratio",
      "color":"5","vendor":{"collapsed":false,"style":"striped"}},
    {"id":"text","type":"text","x":40,"y":60,"width":240,"height":150,
      "text":"# Plan\n- [ ] Check file","color":"#ff0066",
      "extension":{"tags":["one","two"]}},
    {"id":"file","type":"file","x":320,"y":55,"width":240,"height":180,
      "file":"Notes/Chapter.md","subpath":"#Introduction","color":"1"},
    {"id":"link","type":"link","x":330,"y":260,"width":240,"height":130,
      "url":"https://example.org/doc?lang=en","color":"#11223344"}
  ],
  "edges": [
    {"id":"edge-a","fromNode":"text","fromSide":"right","toNode":"file",
      "toSide":"left","toEnd":"arrow","color":"6","label":"references",
      "extension":{"controlPoints":[[5,10],[15,20]]}},
    {"id":"edge-b","fromNode":"file","fromEnd":"none","toNode":"link",
      "toEnd":"arrow","color":"#f0f"}
  ],
  "vendor":{"source":"synthetic","nested":{"v":4}},
  "unusedExtensions":[{"neverDiscard":true}]
}"#;

fn fixture() -> (tempfile::TempDir, PathBuf, Arc<Core>, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    let data = dir.path().join("data");
    fs::create_dir(&vault).unwrap();
    fs::create_dir(&data).unwrap();
    let core = Core::open(&data, Arc::new(|_: CoreEvent| {}));
    core.ui_state.resolve_workspace(vault.to_str().unwrap(), 1).unwrap();
    let path = vault.join("standard.canvas");
    let raw = SPEC_CANVAS.replace('\n', "\r\n").into_bytes();
    fs::write(&path, &raw).unwrap();
    (dir, path, core, raw)
}

fn disk(path: &std::path::Path) -> Value {
    StructuredDocument::open(path).unwrap().canvas().unwrap()
}

#[test]
fn all_four_standard_node_types_and_edge_styles_are_read_without_source_changes() {
    let (_dir, path, _core, original) = fixture();
    let doc = StructuredDocument::open(&path).unwrap();
    let canvas = doc.canvas().unwrap();
    let nodes = canvas["nodes"].as_array().unwrap();
    let kinds: Vec<_> = nodes.iter().map(|v| v["type"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["group", "text", "file", "link"]);
    assert_eq!(nodes[0]["backgroundStyle"], "ratio");
    assert_eq!(nodes[0]["color"], "5");
    assert_eq!(nodes[1]["color"], "#ff0066");
    assert_eq!(nodes[2]["subpath"], "#Introduction");
    assert_eq!(nodes[3]["url"], "https://example.org/doc?lang=en");
    assert_eq!(canvas["edges"][0]["label"], "references");
    assert_eq!(canvas["edges"][1]["color"], "#f0f");
    assert_eq!(canvas["vendor"]["nested"]["v"], 4);
    assert_eq!(fs::read(path).unwrap(), original, "read-only open must be byte-exact");
}

#[test]
fn text_edit_preserves_file_link_group_z_order_extensions_and_labeled_edges() {
    let (_dir, path, core, original) = fixture();
    let before = disk(&path);
    let mut session = CanvasSession::open(&path).unwrap();
    assert!(session.move_node("text", 80, -12).unwrap());
    assert!(session.set_text("text", "# Revised plan").unwrap());
    assert_eq!(fs::read(&path).unwrap(), original, "staged edits never write early");
    assert!(session.undo().unwrap());
    assert_eq!(session.value()["nodes"][1]["text"], "# Plan\n- [ ] Check file");
    assert!(session.redo().unwrap());
    assert!(session.save(&core).unwrap());
    assert_eq!(disk(&path)["nodes"][1]["x"], 80);
    assert_eq!(disk(&path)["nodes"][1]["y"], -12);
    assert_eq!(disk(&path)["nodes"][1]["text"], "# Revised plan");
    let after = disk(&path);
    for index in [0, 2, 3] {
        assert_eq!(after["nodes"][index], before["nodes"][index], "unrelated node {index} changed");
    }
    assert_eq!(after["nodes"][1]["extension"], before["nodes"][1]["extension"]);
    assert_eq!(after["edges"], before["edges"], "connection side, label and metadata must survive");
    assert_eq!(after["vendor"], before["vendor"]);
    assert_eq!(after["unusedExtensions"], before["unusedExtensions"]);
    assert!(!session.save(&core).unwrap(), "second save must be inert");
}

#[test]
fn external_edit_during_standard_format_edit_fails_closed_without_overwrite() {
    let (_dir, path, core, _original) = fixture();
    let mut session = CanvasSession::open(&path).unwrap();
    assert!(session.move_node("text", 140, 110).unwrap());
    let external = SPEC_CANVAS.replace("Research", "External editor");
    fs::write(&path, &external).unwrap();
    assert!(matches!(session.save(&core), Err(CanvasSessionError::Document(
        StructuredError::File(FileCommandError::Conflict { .. })
    ))));
    assert_eq!(fs::read_to_string(&path).unwrap(), external);
    assert!(session.dirty() && session.can_undo(), "the user must retain unsaved local edits");
    assert_eq!(session.value()["nodes"][1]["x"], 140);
    assert!(session.undo().unwrap());
    assert!(!session.dirty());
}

#[test]
fn future_node_and_unknown_fields_survive_a_supported_edit_unchanged() {
    let (_dir, path, core, _original) = fixture();
    let extension = json!({"type":"future-widget","id":"future","x":710,"y":75,
        "width":180,"height":160,"rendererData":{"opaque":[4,2],"active":true}});
    let mut source = disk(&path);
    source["nodes"].as_array_mut().unwrap().push(extension.clone());
    fs::write(&path, serde_json::to_string_pretty(&source).unwrap()).unwrap();
    let mut session = CanvasSession::open(&path).unwrap();
    assert!(session.move_node("text", 111, 222).unwrap());
    assert!(session.save(&core).unwrap());
    let after = disk(&path);
    assert_eq!(after["nodes"][4], extension);
    assert_eq!(after["edges"], source["edges"]);
    assert_eq!(after["vendor"], source["vendor"]);
}
