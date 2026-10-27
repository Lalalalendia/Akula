//! Windows release handoff is reachable only with an authenticated SignedCandidate.
//! Refuse the installer handoff unless its immutable retry ticket is durable.

use super::{attempts, release, Status, Updater};
use std::path::Path;

impl Updater {
    pub(crate) fn take_ready(&self, data_dir: &Path) -> Option<release::SignedCandidate> {
        if !matches!(self.status(), Status::Ready(_)) {
            return None;
        }
        let candidate = self.payload.lock().unwrap_or_else(|e| e.into_inner()).take()?;
        if let Err(error) = attempts::record(data_dir, candidate.version(), candidate.sha256()) {
            self.fail(error);
            return None;
        }
        self.set(Status::Installing);
        Some(candidate)
    }
}
