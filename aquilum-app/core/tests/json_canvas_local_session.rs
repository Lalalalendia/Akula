//! C4 local-only Canvas editing: real File Gate and conflict tests.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aquilum_core::CoreEvent;
use aquilum_core::app_core::Core;
use aquilum_core::files::structured::{StructuredDocument, StructuredError};
use aquilum_core::files::structured::session::{CanvasSession, CanvasSessionError};
use aquilum_core::files::error::FileCommandError;
use serde_json::json;

const ORIGINAL: &str = r#"{"nodes":[{"id":"a","type":"text","x":12,"y":7,"width":200,"height":80,"text":"before","plugin":{"keep":true}},{"id":"b","type":"group","x":0,"y":0,"width":360,"height":220,"label":"group"}],"edges":[{"id":"e","fromNode":"a","toNode":"b","toEnd":"arrow"}],"vendor":{"unknown":[1,2,3]}}"#;

fn setup() -> (tempfile::TempDir, PathBuf, Arc<Core>) {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    let data = dir.path().join("data");
    fs::create_dir(&vault).unwrap();
    fs::create_dir(&data).unwrap();
    let core = Core::open(&data, Arc::new(|_: CoreEvent| {}));
    core.ui_state.resolve_workspace(vault.to_str().unwrap(), 1).unwrap();
    let path = vault.join("board.canvas");
    fs::write(&path, ORIGINAL).unwrap();
    (dir, path, core)
}

fn disk(path: &Path) -> serde_json::Value {
    StructuredDocument::open(path).unwrap().canvas().unwrap()
}

#[test]
fn add_move_text_undo_redo_and_guarded_save_preserve_vendor_data() {
    let (_dir, path, core) = setup();
    let mut session = CanvasSession::open(&path).unwrap();
    assert!(!session.dirty());
    assert!(!session.can_undo());
    assert!(!session.move_node("a", 12, 7).unwrap());
    let created = session.add_text("new", 300, 130).unwrap();
    assert!(session.move_node("a", 220, -30).unwrap());
    assert!(session.set_text("a", "changed").unwrap());
    assert!(session.can_undo() && !session.can_redo());
    assert_eq!(fs::read_to_string(&path).unwrap(), ORIGINAL, "uncommitted changes never reach disk");
    assert_eq!(session.value()["nodes"][0]["x"], 220);
    assert_eq!(session.value()["nodes"][0]["y"], -30);
    assert_eq!(session.value()["nodes"][2]["id"], created);
    assert_eq!(session.value()["vendor"]["unknown"], json!([1, 2, 3]));
    assert!(session.undo().unwrap());
    assert_eq!(session.value()["nodes"][0]["text"], "before");
    assert!(session.redo().unwrap());
    assert_eq!(session.value()["nodes"][0]["text"], "changed");
    assert!(session.save(&core).unwrap());
    assert!(!session.dirty());
    assert_eq!(disk(&path)["nodes"][0]["plugin"]["keep"], true);
    assert_eq!(disk(&path)["nodes"][0]["x"], 220);
    assert_eq!(disk(&path)["nodes"][0]["y"], -30);
    assert_eq!(disk(&path)["nodes"][0]["text"], "changed");
    assert_eq!(disk(&path)["nodes"][2]["id"], created);
    assert!(!session.save(&core).unwrap(), "duplicate save must be a no-op");
}

#[test]
fn deleted_node_and_its_edges_restore_in_one_undo() {
    let (_dir, path, _core) = setup();
    let mut session = CanvasSession::open(&path).unwrap();
    assert!(session.remove_node("a").unwrap());
    assert_eq!(session.value()["nodes"].as_array().unwrap().len(), 1);
    assert!(session.value()["edges"].as_array().unwrap().is_empty());
    assert!(session.undo().unwrap());
    assert_eq!(session.value()["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(session.value()["edges"][0]["id"], "e");
    assert!(!session.dirty());
    assert_eq!(fs::read_to_string(&path).unwrap(), ORIGINAL);
}

#[test]
fn external_write_conflict_keeps_local_edits_and_original_file() {
    let (_dir, path, core) = setup();
    let mut session = CanvasSession::open(&path).unwrap();
    assert!(session.move_node("a", 65, 90).unwrap());
    fs::write(&path, r#"{"nodes":[],"external":true}"#).unwrap();
    let failed = session.save(&core);
    assert!(matches!(failed, Err(CanvasSessionError::Document(
        StructuredError::File(FileCommandError::Conflict { .. })
    ))));
    assert!(session.dirty());
    assert!(session.can_undo());
    assert_eq!(session.value()["nodes"][0]["x"], 65);
    assert_eq!(disk(&path)["external"], true);
    assert!(session.undo().unwrap());
    assert_eq!(session.value()["nodes"][0]["x"], 12);
}

#[test]
fn fractional_legacy_board_is_readable_but_cannot_be_rewritten() {
    let (_dir, path, _core) = setup();
    let legacy = r#"{"nodes":[{"id":"a","type":"text","x":12.125,"y":0,"width":210,"height":50,"text":"legacy"}],"ext":{"keep":true}}"#;
    fs::write(&path, legacy).unwrap();
    let mut session = CanvasSession::open(&path).unwrap();
    assert_eq!(session.value()["nodes"][0]["x"], json!(12.125));
    assert!(matches!(session.move_node("a", 10, 20), Err(CanvasSessionError::Document(_))));
    assert!(matches!(session.add_text("new", 20, 30), Err(CanvasSessionError::Document(_))));
    assert!(!session.dirty());
    assert!(!session.can_undo());
    assert_eq!(fs::read_to_string(&path).unwrap(), legacy);
}

#[test]
fn invalid_actions_cannot_mutate_document_or_history() {
    let (_dir, path, _core) = setup();
    let mut session = CanvasSession::open(&path).unwrap();
    assert!(matches!(session.move_node("missing", 1, 1), Err(CanvasSessionError::MissingNode(_))));
    assert!(matches!(session.set_text("b", "wrong",), Err(CanvasSessionError::NotTextNode(_))));
    assert!(matches!(session.move_node("a", i64::MAX, 4), Err(CanvasSessionError::CoordinateOutOfRange)));
    assert!(!session.can_undo());
    assert!(!session.dirty());
    assert_eq!(fs::read_to_string(&path).unwrap(), ORIGINAL);
}

#[test]
fn clean_session_refreshes_external_revision_and_invalidates_old_history() {
    let (_dir, path, core) = setup();
    let mut session = CanvasSession::open(&path).unwrap();
    session.move_node("a", 33, 44).unwrap();
    session.save(&core).unwrap();
    assert!(session.can_undo());
    let new_doc = r#"{"nodes":[{"id":"new","type":"text","x":5,"y":7,"width":90,"height":40,"text":"external"}],"external":true}"#;
    fs::write(&path, new_doc).unwrap();
    assert!(session.refresh_if_clean().unwrap());
    assert!(!session.dirty());
    assert!(!session.can_undo());
    assert_eq!(session.value()["nodes"][0]["id"], "new");
    assert_eq!(fs::read_to_string(&path).unwrap(), new_doc);
}

#[test]
fn bounded_resize_undo_redo_and_vendor_fields_survive_guarded_save() {
    let (_dir, path, core) = setup();
    let mut session = CanvasSession::open(&path).unwrap();
    let before = session.value().clone();
    assert!(!session.resize_node("a", 200, 80).unwrap(), "same size must be a no-op");
    assert!(session.resize_node("a", 340, 150).unwrap());
    assert_eq!(session.value()["nodes"][0]["width"], 340);
    assert_eq!(session.value()["nodes"][0]["height"], 150);
    assert_eq!(session.value()["nodes"][0]["plugin"]["keep"], true);
    assert_eq!(session.value()["vendor"]["unknown"], json!([1, 2, 3]));
    assert!(session.undo().unwrap());
    assert_eq!(session.value(), &before);
    assert!(session.redo().unwrap());
    assert_eq!(session.value()["nodes"][0]["width"], 340);
    assert!(session.save(&core).unwrap());
    assert_eq!(disk(&path)["nodes"][0]["width"], 340);
    assert_eq!(disk(&path)["vendor"]["unknown"], json!([1, 2, 3]));
}

#[test]
fn invalid_dimensions_and_nontext_resizes_leave_history_and_file_unchanged() {
    let (_dir, path, _core) = setup();
    let mut session = CanvasSession::open(&path).unwrap();
    for (id, w, h) in [
        ("a", 47, 90), ("a", 50, 31), ("a", i64::MAX, 90),
        ("a", 90, i64::MAX), ("missing", 80, 80), ("b", 80, 80),
    ] {
        assert!(session.resize_node(id, w, h).is_err());
        assert!(!session.can_undo());
        assert!(!session.dirty());
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), ORIGINAL);
}

#[test]
fn external_conflict_keeps_resized_state_and_prevents_overwrite() {
    let (_dir, path, core) = setup();
    let mut session = CanvasSession::open(&path).unwrap();
    assert!(session.resize_node("a", 420, 210).unwrap());
    fs::write(&path, r#"{"nodes":[],"external":true}"#).unwrap();
    assert!(matches!(session.save(&core), Err(CanvasSessionError::Document(
        StructuredError::File(FileCommandError::Conflict { .. })
    ))));
    assert_eq!(session.value()["nodes"][0]["width"], 420);
    assert!(session.dirty() && session.can_undo());
    assert_eq!(disk(&path)["external"], true);
}

#[test]
fn add_edge_preserves_vendor_fields_and_round_trips_undo_redo() {
    let (_dir, path, core) = setup();
    let mut session = CanvasSession::open(&path).unwrap();
    let original = session.value().clone();
    let id = session.add_edge("a", "b").unwrap();
    assert_eq!(session.value()["edges"].as_array().unwrap().len(), 2);
    assert_eq!(session.value()["edges"][1]["id"], id);
    assert_eq!(session.value()["edges"][1]["fromNode"], "a");
    assert_eq!(session.value()["edges"][1]["toNode"], "b");
    assert_eq!(session.value()["edges"][1]["toEnd"], "arrow");
    assert_eq!(session.value()["edges"][0], original["edges"][0]);
    assert_eq!(session.value()["vendor"]["unknown"], json!([1,2,3]));
    assert!(session.undo().unwrap());
    assert_eq!(session.value(), &original);
    assert!(session.redo().unwrap());
    assert_eq!(session.value()["edges"][1]["id"], id);
    assert!(session.save(&core).unwrap());
    assert_eq!(disk(&path)["edges"][1]["id"], id);
}

#[test]
fn add_edge_rejects_missing_endpoints_without_mutating_disk_or_history() {
    let (_dir, path, _core) = setup();
    let mut session = CanvasSession::open(&path).unwrap();
    for (from, to) in [("missing", "b"), ("a", "missing"), ("", "b")] {
        assert!(matches!(session.add_edge(from, to), Err(CanvasSessionError::MissingNode(_))));
        assert!(!session.dirty() && !session.can_undo());
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), ORIGINAL);
}

#[test]
fn add_edge_file_gate_conflict_retains_dirty_history_without_overwrite() {
    let (_dir, path, core) = setup();
    let mut session = CanvasSession::open(&path).unwrap();
    let id = session.add_edge("b", "a").unwrap();
    fs::write(&path, r#"{"nodes":[],"vendor":{"external":true}}"#).unwrap();
    assert!(matches!(session.save(&core), Err(CanvasSessionError::Document(
        StructuredError::File(FileCommandError::Conflict { .. })
    ))));
    assert!(session.dirty() && session.can_undo());
    assert_eq!(session.value()["edges"][1]["id"], id);
    assert_eq!(disk(&path)["vendor"]["external"], true);
}

#[test]
fn add_edge_initializes_optional_edges_array_without_data_loss() {
    let (_dir, path, core) = setup();
    fs::write(&path, r#"{"nodes":[{"id":"a","type":"text","text":"a","x":0,"y":0,"width":100,"height":40},{"id":"b","type":"text","text":"b","x":140,"y":0,"width":100,"height":40}],"addon":{"unchanged":17}}"#).unwrap();
    let mut session = CanvasSession::open(&path).unwrap();
    let id = session.add_edge("a", "b").unwrap();
    assert_eq!(session.value()["edges"][0]["id"], id);
    assert!(session.save(&core).unwrap());
    assert_eq!(disk(&path)["addon"]["unchanged"], 17);
}
