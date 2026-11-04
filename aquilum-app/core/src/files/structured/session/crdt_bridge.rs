//! Opt-in, in-memory handoff from the offline Yrs lab to the existing Canvas editor.
//! No transport, no automatic disk writes, and no replacement of File Gate.

use super::{CanvasSession, CanvasSessionError};
use super::super::StructuredError;
use crate::documents::replica::canvas_crdt_lab::CanvasCrdtLab;

impl CanvasSession {
    /// Stage one complete, validated CRDT projection as a normal undoable edit.
    ///
    /// Never discard unsaved local changes. Never refresh an externally edited
    /// disk baseline here: the existing File Gate CAS must detect that conflict
    /// on an explicitly requested save, leaving recovery possible.
    pub fn stage_crdt_projection(
        &mut self, replica: &CanvasCrdtLab,
    ) -> Result<bool, CanvasSessionError> {
        if self.dirty() {
            return Err(CanvasSessionError::Document(StructuredError::InvalidFormat(
                "refusing CRDT projection over unsaved local Canvas edits".into(),
            )));
        }
        let snapshot = replica.snapshot().map_err(|reason| {
            CanvasSessionError::Document(StructuredError::InvalidFormat(reason))
        })?;
        // The established session validates the FULL JSON Canvas and its
        // legacy-lossless constraints before changing Value or undo/redo.
        self.stage(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::{Core, CoreEvent};
    use crate::documents::merge::TextEdit;
    use crate::files::structured::StructuredDocument;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use yrs::ClientID;

    const INITIAL: &str = r#"{
      "nodes": [
        {"id": "t", "type": "text", "x": 0, "y": 0,
         "width": 240, "height": 120, "text": "initial",
         "vendor": {"preserve": [1, 2, 3]}}
      ],
      "edges": [],
      "custom-root": {"keep": true}
    }"#;

    fn fixture() -> (tempfile::TempDir, Arc<Core>, PathBuf, CanvasSession, Vec<u8>) {
        let temp = tempfile::tempdir().unwrap();
        let vault = temp.path().join("vault");
        let appdata = temp.path().join("appdata");
        fs::create_dir(&vault).unwrap();
        fs::create_dir(&appdata).unwrap();
        let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(vault.to_str().unwrap(), 1).unwrap();
        let path = vault.join("board.canvas");
        fs::write(&path, INITIAL).unwrap();
        let original = fs::read(&path).unwrap();
        let session = CanvasSession::open(&path).unwrap();
        (temp, core, path, session, original)
    }

    fn seed(session: &CanvasSession) -> CanvasCrdtLab {
        CanvasCrdtLab::seeded(ClientID::new(101), session.value()).unwrap()
    }

    #[test]
    fn identical_projection_is_a_noop_and_keeps_original_bytes() {
        let (_temp, core, path, mut session, original) = fixture();
        let replica = seed(&session);
        assert!(!session.stage_crdt_projection(&replica).unwrap());
        assert!(!session.dirty());
        assert!(!session.can_undo());
        assert!(!session.can_redo());
        assert_eq!(fs::read(path).unwrap(), original);
        core.shutdown();
    }

    #[test]
    fn staged_projection_uses_existing_undo_redo_and_file_gate_save() {
        let (_temp, core, path, mut session, original) = fixture();
        let replica = seed(&session);
        replica.move_node("t", 300, -125).unwrap();
        replica.edit_text("t", &[TextEdit {
            from: 0, to: 0, insert: "remote: ".into(),
        }]).unwrap();

        assert!(session.stage_crdt_projection(&replica).unwrap());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(session.dirty());
        assert!(session.can_undo());
        assert_eq!(session.value()["nodes"][0]["x"], 300);
        assert_eq!(session.value()["nodes"][0]["y"], -125);
        assert_eq!(session.value()["nodes"][0]["text"], "remote: initial");
        assert_eq!(session.value()["nodes"][0]["vendor"]["preserve"], json!([1, 2, 3]));
        assert_eq!(session.value()["custom-root"]["keep"], true);

        assert!(session.undo().unwrap());
        assert!(!session.dirty());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(session.redo().unwrap());
        assert!(session.dirty());

        assert!(session.save(&core).unwrap());
        assert!(!session.dirty());
        let stored = StructuredDocument::open(&path).unwrap().canvas().unwrap();
        assert_eq!(stored, *session.value());
        assert_eq!(stored["nodes"][0]["vendor"]["preserve"], json!([1, 2, 3]));
        assert!(!session.save(&core).unwrap());
        core.shutdown();
    }

    #[test]
    fn unsaved_local_text_is_not_overwritten_by_remote_projection() {
        let (_temp, core, path, mut session, original) = fixture();
        let replica = seed(&session);
        replica.move_node("t", 400, 500).unwrap();
        session.set_text("t", "UNSAVED LOCAL").unwrap();
        let before = session.value().clone();

        assert!(session.stage_crdt_projection(&replica).is_err());
        assert_eq!(session.value(), &before);
        assert!(session.dirty());
        assert!(session.can_undo());
        assert!(!session.can_redo());
        assert_eq!(fs::read(path).unwrap(), original);
        core.shutdown();
    }

    #[test]
    fn external_disk_change_still_causes_file_gate_conflict() {
        let (_temp, core, path, mut session, _original) = fixture();
        let replica = seed(&session);
        replica.move_node("t", 40, 60).unwrap();
        assert!(session.stage_crdt_projection(&replica).unwrap());
        let external = r#"{"nodes":[],"edges":[],"external":{"keep":true}}"#;
        fs::write(&path, external).unwrap();

        assert!(session.save(&core).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), external);
        assert_eq!(session.value()["nodes"][0]["x"], 40);
        assert_eq!(session.value()["nodes"][0]["y"], 60);
        assert!(session.dirty());
        assert!(session.can_undo());
        core.shutdown();
    }

    #[test]
    fn invalid_canvas_projection_cannot_publish_or_change_history() {
        let (_temp, core, path, mut session, original) = fixture();
        // The experimental Yrs seed is intentionally NOT a full JSON Canvas
        // schema gate: the established StructuredDocument gate must reject it.
        let invalid = json!({
            "nodes": [{
                "id": "t", "type": "text", "x": 0, "y": 0,
                "width": 240, "height": 120, "text": "unsafe",
                "color": true
            }],
            "edges": []
        });
        let replica = CanvasCrdtLab::seeded(ClientID::new(102), &invalid).unwrap();
        let before = session.value().clone();
        assert!(session.stage_crdt_projection(&replica).is_err());
        assert_eq!(session.value(), &before);
        assert!(!session.dirty());
        assert!(!session.can_undo());
        assert!(!session.can_redo());
        assert_eq!(fs::read(&path).unwrap(), original);
        core.shutdown();
    }

    #[test]
    fn legacy_fractional_geometry_remains_read_only() {
        let (_temp, core, path, _session, _original) = fixture();
        let legacy = r#"{"nodes":[{"id":"t","type":"text","x":0.5,"y":0,
            "width":240,"height":120,"text":"legacy"}],"edges":[]}"#;
        fs::write(&path, legacy).unwrap();
        let mut session = CanvasSession::open(&path).unwrap();
        let replica = seed(&session);
        replica.move_node("t", 1, 2).unwrap();

        assert!(session.stage_crdt_projection(&replica).is_err());
        assert!(!session.dirty());
        assert!(!session.can_undo());
        assert_eq!(fs::read_to_string(&path).unwrap(), legacy);
        core.shutdown();
    }

    #[test]
    fn offline_two_peer_merge_projects_without_early_disk_write() {
        let (_temp, core, path, mut session, original) = fixture();
        let a = seed(&session);
        let b = CanvasCrdtLab::new(ClientID::new(202));
        b.apply_remote_update(&a.state()).unwrap();
        a.move_node("t", 9, -11).unwrap();
        b.edit_text("t", &[TextEdit {
            from: 0, to: 0, insert: "peer: ".into(),
        }]).unwrap();
        a.apply_remote_update(&b.state()).unwrap();

        assert!(session.stage_crdt_projection(&a).unwrap());
        assert_eq!(session.value()["nodes"][0]["x"], 9);
        assert_eq!(session.value()["nodes"][0]["y"], -11);
        assert_eq!(session.value()["nodes"][0]["text"], "peer: initial");
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(!session.stage_crdt_projection(&a).is_ok());
        assert_eq!(fs::read(&path).unwrap(), original);
        core.shutdown();
    }
}
