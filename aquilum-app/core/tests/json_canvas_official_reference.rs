//! C5 official-reference JSON Canvas 1.0 interoperability tests.
//! Fixture from obsidianmd/jsoncanvas sample.canvas at 8ca9fdcac7292a1b44480659d891772a0fa891f7.
//! MIT (c) 2024 Obsidian.md: see fixtures/jsoncanvas/LICENSE.
//! This verifies real published reference bytes, not an Obsidian GUI round trip.
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use aquilum_core::CoreEvent;
use aquilum_core::app_core::Core;
use aquilum_core::files::error::FileCommandError;
use aquilum_core::files::structured::{StructuredDocument, StructuredError};
use aquilum_core::files::structured::session::{CanvasSession, CanvasSessionError};
use serde_json::Value;

const REFERENCE: &str = include_str!("fixtures/jsoncanvas/official-sample.canvas");
const TEXT_ID: &str = "59e896bc8da20699";
const FILE_ID: &str = "8132d4d894c80022";

fn setup() -> (tempfile::TempDir, PathBuf, Arc<Core>, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    let data = dir.path().join("data");
    fs::create_dir(&vault).unwrap();
    fs::create_dir(&data).unwrap();
    let core = Core::open(&data, Arc::new(|_: CoreEvent| {}));
    core.ui_state.resolve_workspace(vault.to_str().unwrap(), 1).unwrap();
    let path = vault.join("official.canvas");
    let raw = REFERENCE.as_bytes().to_vec();
    fs::write(&path, &raw).unwrap();
    (dir, path, core, raw)
}

fn parse(path: &std::path::Path) -> Value {
    StructuredDocument::open(path).unwrap().canvas().unwrap()
}

#[test]
fn official_reference_open_is_byte_exact_with_unresolved_file_references() {
    let (_dir, path, _core, source) = setup();
    let canvas = parse(&path);
    let nodes = canvas["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 5);
    assert_eq!(canvas["edges"].as_array().unwrap().len(), 1);
    assert_eq!(nodes[0]["type"], "group");
    assert_eq!(nodes[1]["file"], "readme.md");
    assert_eq!(nodes[2]["file"], "_site/logo.svg");
    assert_eq!(nodes[3]["type"], "text");
    assert_eq!(nodes[4]["file"], "spec/1.0.md");
    assert_eq!(nodes[3]["text"].as_str().unwrap().contains("- [Apps]"), true);
    assert_eq!(canvas["edges"][0]["fromSide"], "right");
    assert_eq!(canvas["edges"][0]["toSide"], "left");
    assert!(canvas["edges"][0].get("toEnd").is_none(), "optional default is absent");
    let session = CanvasSession::open(&path).unwrap();
    assert_eq!(session.value(), &canvas);
    assert!(!session.dirty());
    assert_eq!(fs::read(&path).unwrap(), source, "opening may not rewrite whitespace or ordering");
}

#[test]
fn official_reference_text_edit_preserves_files_z_order_and_original_connection() {
    let (_dir, path, core, bytes) = setup();
    let before = parse(&path);
    let mut session = CanvasSession::open(&path).unwrap();
    assert!(session.move_node(TEXT_ID, 41, -431).unwrap());
    assert!(session.set_text(TEXT_ID, "Akula edited Markdown card").unwrap());
    assert_eq!(fs::read(&path).unwrap(), bytes, "staging must not touch source");
    assert!(session.undo().unwrap());
    assert_eq!(session.value()["nodes"][3]["text"], before["nodes"][3]["text"]);
    assert!(session.redo().unwrap());
    assert!(session.save(&core).unwrap());
    let after = parse(&path);
    assert_eq!(after["nodes"][3]["x"], 41);
    assert_eq!(after["nodes"][3]["y"], -431);
    assert_eq!(after["nodes"][3]["text"], "Akula edited Markdown card");
    for index in [0, 1, 2, 4] {
        assert_eq!(after["nodes"][index], before["nodes"][index], "unrelated node {index} changed");
    }
    assert_eq!(after["edges"], before["edges"], "original sides and optional ends preserved");
    assert_eq!(after["nodes"][1]["file"], "readme.md");
    assert_eq!(after["nodes"][2]["file"], "_site/logo.svg");
    assert_eq!(after["nodes"][4]["file"], "spec/1.0.md");
    let reopened = CanvasSession::open(&path).unwrap();
    assert_eq!(reopened.value(), &after);
    assert!(!reopened.dirty());
}

#[test]
fn official_reference_group_and_edge_creation_preserve_every_original_element() {
    let (_dir, path, core, bytes) = setup();
    let original = parse(&path);
    let mut session = CanvasSession::open(&path).unwrap();
    let group = session.add_group("Review", -380, -510).unwrap();
    let edge = session.add_edge(TEXT_ID, FILE_ID).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(session.value()["nodes"][0]["id"], group);
    assert_eq!(session.value()["nodes"][0]["type"], "group");
    assert_eq!(session.value()["edges"][1]["id"], edge);
    assert_eq!(session.value()["edges"][1]["toEnd"], "arrow");
    assert!(session.undo().unwrap());
    assert_eq!(session.value()["edges"], original["edges"]);
    assert!(session.redo().unwrap());
    assert!(session.save(&core).unwrap());
    let after = parse(&path);
    assert_eq!(after["nodes"].as_array().unwrap().len(), 6);
    assert_eq!(after["edges"].as_array().unwrap().len(), 2);
    for (index, node) in original["nodes"].as_array().unwrap().iter().enumerate() {
        assert_eq!(&after["nodes"][index + 1], node, "original node lost or reordered");
    }
    assert_eq!(after["edges"][0], original["edges"][0]);
    assert_eq!(after["edges"][1]["fromNode"], TEXT_ID);
    assert_eq!(after["edges"][1]["toNode"], FILE_ID);
}

#[test]
fn external_reference_edit_during_akula_session_is_rejected_without_loss() {
    let (_dir, path, core, _bytes) = setup();
    let mut session = CanvasSession::open(&path).unwrap();
    assert!(session.move_node(TEXT_ID, 95, -390).unwrap());
    let external = REFERENCE.replace("JSON Canvas", "Edited in another application");
    fs::write(&path, &external).unwrap();
    assert!(matches!(session.save(&core), Err(CanvasSessionError::Document(
        StructuredError::File(FileCommandError::Conflict { .. })
    ))));
    assert_eq!(fs::read_to_string(&path).unwrap(), external);
    assert!(session.dirty() && session.can_undo());
    assert_eq!(session.value()["nodes"][3]["x"], 95);
    assert!(session.undo().unwrap());
    assert!(!session.dirty());
}
