//! Live-study admission for the currently visible native Note buffer.
//!
//! The window already owns the editor text. This only reports whether that
//! in-memory buffer has a safe, stable revision to project; it never opens
//! a file, mints a document ID, or clears a failed/uncertain save.

use std::sync::PoisonError;

use super::Note;

impl Note {
    /// Allow debounced, unsaved edits; refuse conflicted, uncertain, loading
    /// or in-flight saves. Include the file baseline digest so a completed
    /// save or reload invalidates any previously prepared study exercise.
    pub(crate) fn study_live_stamp(&self) -> Option<(u64, String)> {
        if self.save_safety.held()
            || self.save_error.is_some()
            || self.replaced.is_some()
            || self.reload_job.is_some()
            || self.queued != 0
        {
            return None;
        }
        let baseline = self.hash.lock().unwrap_or_else(PoisonError::into_inner).clone();
        Some((self.revision, baseline))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use super::*;

    #[test]
    fn unsaved_editor_edits_have_a_live_revision_without_file_io() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("learning.md");
        std::fs::write(&path, "| Q | A |\n| --- | --- |\n| go | идти |").unwrap();
        let (mut note, _) = Note::open(path.clone()).unwrap();
        let original = note.study_live_stamp().unwrap();
        note.edit(Duration::from_secs(60));
        let edited = note.study_live_stamp().unwrap();
        assert_ne!(edited.0, original.0);
        assert_eq!(edited.1, original.1);
        assert_eq!(std::fs::read_to_string(path).unwrap(),
            "| Q | A |\n| --- | --- |\n| go | идти |");
    }

    #[test]
    fn conflicted_and_in_flight_saves_are_never_study_authority() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("learning.md");
        std::fs::write(&path, "test").unwrap();
        let (mut note, _) = Note::open(path).unwrap();
        assert!(note.study_live_stamp().is_some());
        note.queued = 1;
        assert!(note.study_live_stamp().is_none());
        note.queued = 0;
        note.save_error = Some("save needs review".into());
        assert!(note.study_live_stamp().is_none());
        note.save_error = None;
        note.replaced = Some(crop::Rope::from("new disk text"));
        assert!(note.study_live_stamp().is_none());
    }
}
