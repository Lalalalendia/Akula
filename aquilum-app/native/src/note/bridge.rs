//! Controlled bridge from Note to the DESK-15 snapshot sink.
//! Live editor edits are captured; a confirmed File Gate save retires only matching shadows.
mod lifecycle;
use std::io;
use std::path::Path;

use crop::Rope;
use uuid::Uuid;

use super::Note;
use super::recovery::{CaptureStatus, SnapshotQueue};
pub(crate) use super::recovery::has_unreviewed_shadow_under_move;

/// Read-only candidate. No automatic write to user documents is possible here.
pub(crate) struct RecoveryCandidate {
    pub(crate) revision: u64,
    pub(crate) body: Rope,
    pub(crate) baseline: String,
    pub(crate) source_changed: bool,
    pub(crate) already_saved: bool,
    pub(crate) backup_path: std::path::PathBuf,
}

pub(crate) fn inspect_for_recovery(
    data: &Path, vault: &Path, workspace: Uuid, document: &Path,
) -> io::Result<Option<RecoveryCandidate>> {
    super::recovery::inspect(data, vault, workspace, document).map(|candidate| {
        candidate.map(|record| RecoveryCandidate {
            revision: record.revision,
            body: Rope::from(record.body),
            baseline: record.baseline,
            source_changed: record.source_changed,
            already_saved: record.already_saved,
            backup_path: record.file,
        })
    })
}

impl Note {
    /// Queued != durable. The App must not report recovery as safe until the
    /// separate worker has returned a real revision acknowledgment.
    pub(crate) fn snapshot_unsaved(&self, source: &Rope) -> io::Result<Option<u64>> {
        self.shadow.as_ref().map(|shadow| shadow.request(source.clone())).transpose()
    }

    pub(crate) fn approve_recovery_capture(&self) {
        if let Some(shadow) = &self.shadow { shadow.allow_after_review(); }
    }

    pub(crate) fn saved_ack(&self) {
        if let Some(shadow) = &self.shadow {
            shadow.after_save_ack();
        }
    }

    pub(crate) fn shadow_error(&self) -> Option<String> {
        self.shadow_status().and_then(|status| status.error)
    }

    pub(crate) fn shadow_status(&self) -> Option<CaptureStatus> {
        self.shadow.as_ref().map(SnapshotQueue::status)
    }

    /// A tab switch drops Note while its File Gate save can still be queued.
    /// Keep the ACK receiver alive until those serialized jobs complete, then
    /// run the same verified shadow cleanup as foreground saves. Never clear a
    /// backup for Conflict, Failed or NeedsReview outcomes.
    pub(crate) fn finish_detached_save(self) {
        if self.shadow.is_some() && self.queued > 0 {
            crate::disk::run(move || {
                let mut note = self;
                note.collect();
            });
        }
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
    fn exact_file_gate_save_ack_removes_only_matching_recovery_records() {
        use std::sync::Arc;
        use aquilum_core::Core;

        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let path = vault.path().join("note.md");
        std::fs::write(&path, "disk").unwrap();
        let core = Arc::new(Core::open(data.path(), Arc::new(|_| {})));
        let workspace = Uuid::new_v4();
        let (mut note, _) = Note::open(path.clone()).unwrap();
        note.arm_shadow(data.path(), vault.path(), workspace).unwrap();
        note.snapshot_unsaved(&Rope::from("first")).unwrap();
        let next = Rope::from("second 😀");
        note.snapshot_unsaved(&next).unwrap();
        note.edit(std::time::Duration::ZERO);
        note.flush(&core, &next);
        // File Gate ACK is received synchronously; cleanup is a subsequent
        // verified job on the serial disk worker.
        crate::disk::wait();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second 😀");
        assert!(note.shadow_status().unwrap().error.is_none());
        assert!(super::super::recovery::inspect(data.path(), vault.path(), workspace, &path)
            .unwrap().is_none(), "only exact File Gate ACK may retire shadow snapshots");
        core.shutdown();
    }

    #[test]
    fn conflict_or_external_divergence_never_clears_unsaved_snapshot() {
        use std::sync::Arc;
        use aquilum_core::Core;

        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let path = vault.path().join("note.md");
        std::fs::write(&path, "baseline").unwrap();
        let core = Arc::new(Core::open(data.path(), Arc::new(|_| {})));
        let workspace = Uuid::new_v4();
        let (mut note, _) = Note::open(path.clone()).unwrap();
        note.arm_shadow(data.path(), vault.path(), workspace).unwrap();
        let changed = Rope::from("my unsaved 🐱");
        note.snapshot_unsaved(&changed).unwrap();
        crate::disk::wait();
        std::fs::write(&path, "other process").unwrap();
        note.edit(std::time::Duration::ZERO);
        note.flush(&core, &changed);
        crate::disk::wait();
        let candidate = super::super::recovery::inspect(data.path(), vault.path(), workspace, &path)
            .unwrap().unwrap();
        assert_eq!(candidate.body, "my unsaved 🐱");
        assert!(candidate.source_changed);
        assert!(!candidate.already_saved);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "other process");
        core.shutdown();
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
