//! Native read-side projection and explicit Save Copy from checked Inbox.
//! No automatic network transport, sender ACK, Vault writes or overwrite.
//! An export rehashes the same pinned file descriptor at click time.

use super::super::App;
use crate::ui::transfer::{InboxRow, InboxView};
use akula_inbox_ledger_lab::{InboxLedger, LocalStatus, user_inbox_root};
use aquilum_core::p2p_inbox_lab::save_verified_stage_as_new;
use std::{
    path::Path,
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
};
use uuid::Uuid;

#[derive(Default)]
pub(super) struct InboxPreview {
    pub(super) view: InboxView,
    rx: Option<Receiver<Result<InboxView, String>>>,
    save_rx: Option<Receiver<Result<bool, String>>>,
}

impl InboxPreview {
    fn begin(&mut self, receiver: Receiver<Result<InboxView, String>>) {
        self.rx = Some(receiver);
        self.view.loading = true;
        self.view.error = None;
    }

    pub(super) fn poll(&mut self) -> bool {
        let mut changed = false;
        let next = self.rx.as_ref().map(|receiver| receiver.try_recv());
        match next {
            Some(Ok(Ok(mut view))) => {
                view.exporting = self.view.exporting;
                view.notice_key = self.view.notice_key;
                self.view = view;
                self.rx = None;
                changed = true;
            }
            Some(Ok(Err(error))) => {
                self.view = InboxView {
                    error: Some(error),
                    exporting: self.view.exporting,
                    notice_key: self.view.notice_key,
                    ..InboxView::default()
                };
                self.rx = None;
                changed = true;
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.view.loading = false;
                self.view.error = Some("reader worker stopped".into());
                self.rx = None;
                changed = true;
            }
            _ => {}
        }
        let saved = self.save_rx.as_ref().map(|receiver| receiver.try_recv());
        match saved {
            Some(Ok(Ok(true))) => {
                self.view.notice_key = Some("transfer.inboxSaved");
                self.view.exporting = false;
                self.save_rx = None;
                changed = true;
            }
            Some(Ok(Ok(false))) => {
                self.view.exporting = false;
                self.save_rx = None;
                changed = true;
            }
            Some(Ok(Err(_))) | Some(Err(TryRecvError::Disconnected)) => {
                self.view.notice_key = Some("transfer.inboxSaveFailed");
                self.view.exporting = false;
                self.save_rx = None;
                changed = true;
            }
            _ => {}
        }
        changed
    }
}

impl App {
    pub(super) fn begin_inbox_preview(&mut self) {
        let (tx, rx) = mpsc::channel();
        self.transfer.inbox.begin(rx);
        let wake = Arc::clone(&self.workspace.wake);
        thread::spawn(move || {
            let result = user_inbox_root()
                .map_err(|err| format!("inbox root: {err:?}"))
                .and_then(|root| read_checked_inbox(&root));
            let _ = tx.send(result);
            wake();
        });
    }

    /// A displayed UUID is only a selector, never a filename or permission.
    /// No stale, pending, tampered or duplicate-busy row may start Save Copy.
    pub(super) fn begin_inbox_save(&mut self, receipt: &str) -> bool {
        if self.transfer.inbox.view.exporting || self.transfer.inbox.view.loading {
            return false;
        }
        if !self.transfer.inbox.view.rows.iter().any(|item| {
            item.id == receipt && item.status_key == "transfer.inboxVerified"
        }) {
            return false;
        }
        let Ok(id) = Uuid::parse_str(receipt) else {
            return false;
        };
        let (tx, rx) = mpsc::channel();
        self.transfer.inbox.save_rx = Some(rx);
        self.transfer.inbox.view.exporting = true;
        self.transfer.inbox.view.notice_key = None;
        let wake = Arc::clone(&self.workspace.wake);
        thread::spawn(move || {
            let suggested = format!("received-{id}.bin");
            let chosen = rfd::FileDialog::new()
                .set_file_name(suggested.as_str())
                .save_file();
            let result = if let Some(destination) = chosen {
                user_inbox_root()
                    .map_err(|err| format!("inbox root: {err:?}"))
                    .and_then(|root| save_checked_copy(&root, id, &destination))
                    .map(|()| true)
            } else {
                Ok(false)
            };
            let _ = tx.send(result);
            wake();
        });
        true
    }
}

fn read_checked_inbox(root: &Path) -> Result<InboxView, String> {
    let Some(ledger) = InboxLedger::open_existing(root)
        .map_err(|error| format!("read-only ledger: {error:?}"))?
    else {
        return Ok(InboxView::default());
    };
    let total = ledger
        .count()
        .map_err(|error| format!("count inbox: {error:?}"))?;
    let rows = ledger
        .list_recent(24)
        .map_err(|error| format!("check recent received files: {error:?}"))?
        .into_iter()
        .map(|(file, status)| {
            let id = file.id.hyphenated().to_string();
            let sender: String = file.sender_key[..6]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            InboxRow {
                id,
                bytes: file.bytes,
                sender,
                status_key: match status {
                    LocalStatus::VerifiedLocal => "transfer.inboxVerified",
                    LocalStatus::Pending => "transfer.inboxPending",
                    LocalStatus::NeedsReview => "transfer.inboxReview",
                    LocalStatus::Unknown => "transfer.inboxUnknown",
                },
            }
        })
        .collect();
    Ok(InboxView {
        rows,
        total,
        loading: false,
        error: None,
        exporting: false,
        notice_key: None,
    })
}

fn save_checked_copy(root: &Path, id: Uuid, destination: &Path) -> Result<(), String> {
    let Some(ledger) = InboxLedger::open_existing(root)
        .map_err(|error| format!("read-only ledger: {error:?}"))?
    else {
        return Err("receiver inbox does not exist".into());
    };
    // Recheck the complete same opened file before attempting a new destination.
    let (mut pinned, intent) = ledger
        .open_verified_file(id)
        .map_err(|error| format!("received object invalid: {error:?}"))?;
    save_verified_stage_as_new(
        &mut pinned,
        intent.bytes,
        intent.blake3,
        destination,
    )
    .map_err(|error| format!("Save Copy refused: {error:?}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use akula_inbox_ledger_lab::{ExpectedTransfer, Transport};
    use std::{fs, io::Write};

    #[test]
    fn missing_inbox_never_creates_any_path() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("received");
        let model = read_checked_inbox(&missing).unwrap();
        assert_eq!(model.total, 0);
        assert!(model.rows.is_empty());
        assert!(!missing.exists());
    }

    #[test]
    fn native_projection_reports_verified_and_corrupt_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"read-only native UI physical receipt";
        let file = ExpectedTransfer {
            id: Uuid::new_v4(),
            sender_key: [11; 32],
            recipient_key: [22; 32],
            source: Transport::P2p,
            bytes: bytes.len() as u64,
            blake3: *blake3::hash(bytes).as_bytes(),
        };
        let mut ledger = InboxLedger::open(dir.path()).unwrap();
        assert_eq!(ledger.reserve(&file).unwrap(), LocalStatus::Pending);
        let path = ledger.object_path(file.id);
        let mut created = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .unwrap();
        created.write_all(bytes).unwrap();
        created.sync_all().unwrap();
        drop(created);
        assert_eq!(ledger.confirm(&file).unwrap(), LocalStatus::VerifiedLocal);
        drop(ledger);
        let model = read_checked_inbox(dir.path()).unwrap();
        assert_eq!(model.total, 1);
        assert_eq!(model.rows[0].status_key, "transfer.inboxVerified");

        let copy = dir.path().join("new-copy.bin");
        save_checked_copy(dir.path(), file.id, &copy).unwrap();
        assert_eq!(fs::read(&copy).unwrap(), bytes);
        // No replacing even a user-created or previously exported file.
        assert!(save_checked_copy(dir.path(), file.id, &copy).is_err());
        assert_eq!(fs::read(&copy).unwrap(), bytes);

        fs::write(&path, b"modified").unwrap();
        let model = read_checked_inbox(dir.path()).unwrap();
        assert_eq!(model.total, 1);
        assert_eq!(model.rows[0].status_key, "transfer.inboxReview");
        let refused = dir.path().join("never-created.bin");
        assert!(save_checked_copy(dir.path(), file.id, &refused).is_err());
        assert!(!refused.exists());
    }
}
