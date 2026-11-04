use super::{Status, Updater};

impl Updater {
    // The state must be claimed under one lock before spawning a worker.
    pub(super) fn try_start(&self) -> bool {
        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(
            *status,
            Status::Checking | Status::Downloading { .. } | Status::Ready(_) | Status::Installing
        ) {
            return false;
        }
        *status = Status::Checking;
        drop(status);
        (self.wake)();
        true
    }
}
