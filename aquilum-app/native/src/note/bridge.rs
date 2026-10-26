//! Controlled bridge from Note to the DESK-15 snapshot sink.
//! No automatic runtime hookup until an explicit recovery UX is accepted.
use std::io;
use std::path::Path;

use crop::Rope;
use uuid::Uuid;

use super::Note;
use super::recovery::{CaptureStatus, SnapshotQueue};

impl Note {
    /// Only arm the sink after a verified source read and a real workspace ID.
    pub(crate) fn arm_shadow(&mut self, data: &Path, vault: &Path, workspace: Uuid) -> io::Result<()> {
        if self.shadow.is_none() {
            let baseline = self.hash.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
            self.shadow = Some(SnapshotQueue::new(data, vault, workspace, &self.path, &baseline)?);
        }
        Ok(())
    }

    /// Queued != durable. The App must not report recovery as safe until the
    /// separate worker has returned a real revision acknowledgment.
    pub(crate) fn snapshot_unsaved(&self, source: &Rope) -> io::Result<Option<u64>> {
        self.shadow.as_ref().map(|shadow| shadow.request(source.clone())).transpose()
    }

    pub(crate) fn shadow_status(&self) -> Option<CaptureStatus> {
        self.shadow.as_ref().map(SnapshotQueue::status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_dirty_event_can_stage_without_mutating_file_gate_source() {
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let path = vault.path().join("note.md");
        std::fs::write(&path, "persisted").unwrap();
        let workspace = Uuid::new_v4();

        let (mut note, _original) = Note::open(path.clone()).unwrap();
        note.arm_shadow(data.path(), vault.path(), workspace).unwrap();
        note.edit(std::time::Duration::from_secs(30));
        let revision = note.snapshot_unsaved(&Rope::from("новый текст 😀")).unwrap().unwrap();
        crate::disk::wait();
        assert_eq!(note.shadow_status().unwrap().durable_revision, Some(revision));
        assert!(note.shadow_status().unwrap().error.is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "persisted");

        drop(note); // simulate a lost UI instance before debounce
        let (_reopened, from_disk) = Note::open(path.clone()).unwrap();
        assert_eq!(from_disk.to_string(), "persisted");
        let held = super::super::recovery::inspect(data.path(), vault.path(), workspace, &path)
            .unwrap().unwrap();
        assert_eq!(held.body, "новый текст 😀");
        assert!(!held.already_saved);
        assert!(!held.source_changed);
    }

    #[test]
    fn no_shadow_sink_is_a_noop_for_the_existing_editor_save_path() {
        let vault = tempfile::tempdir().unwrap();
        let path = vault.path().join("original.md");
        std::fs::write(&path, "disk").unwrap();
        let (mut note, _) = Note::open(path).unwrap();
        note.edit(std::time::Duration::ZERO);
        assert_eq!(note.snapshot_unsaved(&Rope::from("dirty")).unwrap(), None);
        assert!(note.shadow_status().is_none());
    }
}
