//! Read-only Native projection of received files from the checked SQLite ledger.
//! The preview never starts networking, creates files, imports into the Vault,
//! considers remote names as paths or issues a delivery acknowledgement.

use super::super::App;
use crate::ui::transfer::{InboxRow, InboxView};
use akula_inbox_ledger_lab::{InboxLedger, LocalStatus, user_inbox_root};
use std::{
    path::Path,
    sync::{Arc, mpsc::{self, Receiver, TryRecvError}},
    thread,
};

#[derive(Default)]
pub(super) struct InboxPreview {
    pub(super) view: InboxView,
    rx: Option<Receiver<Result<InboxView, String>>>,
}

impl InboxPreview {
    fn begin(&mut self, receiver: Receiver<Result<InboxView, String>>) {
        self.rx = Some(receiver);
        self.view.loading = true;
        self.view.error = None;
    }

    pub(super) fn poll(&mut self) -> bool {
        let Some(receiver) = &self.rx else { return false; };
        let next = receiver.try_recv();
        match next {
            Ok(Ok(view)) => { self.view = view; self.rx = None; true }
            Ok(Err(error)) => {
                self.view = InboxView { error: Some(error), ..InboxView::default() };
                self.rx = None;
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.view = InboxView { error: Some("reader worker stopped".into()), ..InboxView::default() };
                self.rx = None;
                true
            }
        }
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
}

fn read_checked_inbox(root: &Path) -> Result<InboxView, String> {
    let Some(ledger) = InboxLedger::open_existing(root)
        .map_err(|error| format!("read-only ledger: {error:?}"))?
    else { return Ok(InboxView::default()); };
    let total = ledger.count().map_err(|error| format!("count inbox: {error:?}"))?;
    let rows = ledger.list_recent(24)
        .map_err(|error| format!("check recent received files: {error:?}"))?
        .into_iter()
        .map(|(file, status)| {
            let id = file.id.hyphenated().to_string();
            let sender: String = file.sender_key[..6].iter()
                .map(|b| format!("{b:02x}")).collect();
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
        }).collect();
    Ok(InboxView { rows, total, loading: false, error: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use akula_inbox_ledger_lab::{ExpectedTransfer, Transport};
    use uuid::Uuid;
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
            sender_key: [11;32],
            recipient_key: [22;32],
            source: Transport::P2p,
            bytes: bytes.len() as u64,
            blake3: *blake3::hash(bytes).as_bytes(),
        };
        let mut ledger = InboxLedger::open(dir.path()).unwrap();
        assert_eq!(ledger.reserve(&file).unwrap(), LocalStatus::Pending);
        let path = ledger.object_path(file.id);
        let mut created = fs::OpenOptions::new().create_new(true).write(true).open(&path).unwrap();
        created.write_all(bytes).unwrap();
        created.sync_all().unwrap();
        drop(created);
        assert_eq!(ledger.confirm(&file).unwrap(), LocalStatus::VerifiedLocal);
        drop(ledger);
        let model = read_checked_inbox(dir.path()).unwrap();
        assert_eq!(model.total, 1);
        assert_eq!(model.rows[0].status_key, "transfer.inboxVerified");
        fs::write(&path, b"modified").unwrap();
        let model = read_checked_inbox(dir.path()).unwrap();
        assert_eq!(model.total, 1);
        assert_eq!(model.rows[0].status_key, "transfer.inboxReview");
    }
}
