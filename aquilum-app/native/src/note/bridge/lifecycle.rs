use super::{Note, SnapshotQueue, inspect_for_recovery};
use std::{io, path::Path};
use uuid::Uuid;

impl Note {
    /// Only arm the sink after a verified source read and a real workspace ID.
    pub(crate) fn arm_shadow(&mut self, data: &Path, vault: &Path, workspace: Uuid) -> io::Result<()> {
        if self.shadow.is_none() {
            let baseline = self.hash.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
            self.shadow = Some(SnapshotQueue::new(data, vault, workspace, &self.path, &baseline)?);
        }
        Ok(())
    }

    /// The rename operation may have rewritten wiki-links inside this note.
    /// Accept the resulting File Gate hash before rearming its recovery queue:
    /// otherwise a subsequent save would falsely conflict with our own move.
    pub(crate) fn accept_verified_rename_hash(&mut self, hash: &str) {
        *self.hash.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = hash.to_owned();
    }

    /// After a successful file/folder move, capture new crash drafts under the
    /// confirmed destination path. The new queue is constructed before replacing
    /// the old one, so a failed rebind remains visible to the caller.
    pub(crate) fn rebind_shadow_after_move(&mut self, data: &Path, vault: &Path, workspace: Uuid) -> io::Result<()> {
        let baseline = self.hash.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        let queue = SnapshotQueue::new(data, vault, workspace, &self.path, &baseline)?;
        self.shadow = Some(queue);
        Ok(())
    }

    /// Do not orphan a previous crash draft by moving its source away before
    /// the user has reviewed it. The caller must settle pending save/snapshot
    /// jobs before this inspection.
    pub(crate) fn has_unreviewed_move_shadow(&self, data: &Path, vault: &Path, workspace: Uuid) -> io::Result<bool> {
        inspect_for_recovery(data, vault, workspace, &self.path)
            .map(|candidate| candidate.is_some_and(|record| !record.already_saved))
    }
}
