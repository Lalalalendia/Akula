//! Native read-side projection and explicit Save Copy from checked Inbox.
//! No automatic network transport, sender ACK, Vault writes or overwrite.
//! An export rehashes the same pinned file descriptor at click time.

use super::super::App;
use crate::ui::transfer::{InboxRow, InboxView};
use akula_inbox_ledger_lab::{InboxLedger, LocalStatus, user_inbox_root};
use aquilum_core::p2p_inbox_lab::save_verified_stage_as_new;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
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
    preview_rx: Option<Receiver<Result<Option<String>, String>>>,
}

impl InboxPreview {
    fn begin(&mut self, receiver: Receiver<Result<InboxView, String>>) {
        self.rx = Some(receiver);
        self.preview_rx = None;
        self.view.loading = true;
        self.view.previewing = false;
        self.view.preview = None;
        self.view.error = None;
    }

    pub(super) fn poll(&mut self) -> bool {
        let mut changed = false;
        let next = self.rx.as_ref().map(|receiver| receiver.try_recv());
        match next {
            Some(Ok(Ok(mut view))) => {
                view.exporting = self.view.exporting;
                view.previewing = self.view.previewing;
                view.preview = self.view.preview.clone();
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
        let previewed = self.preview_rx.as_ref().map(|receiver| receiver.try_recv());
        match previewed {
            Some(Ok(Ok(Some(text)))) => {
                if let Some(id) = self.view.preview.as_ref().map(|(id, _)| id.clone()) {
                    self.view.preview = Some((id, text));
                }
                self.view.previewing = false;
                self.preview_rx = None;
                changed = true;
            }
            Some(Ok(Ok(None))) => {
                self.view.preview = None;
                self.view.notice_key = Some("transfer.inboxPreviewUnsupported");
                self.view.previewing = false;
                self.preview_rx = None;
                changed = true;
            }
            Some(Ok(Err(_))) | Some(Err(TryRecvError::Disconnected)) => {
                self.view.preview = None;
                self.view.notice_key = Some("transfer.inboxPreviewFailed");
                self.view.previewing = false;
                self.preview_rx = None;
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

    /// The preview is user-triggered. A displayed UUID is only a selector,
    /// never a filename, path or authorization to read arbitrary files.
    pub(super) fn begin_inbox_text_preview(&mut self, receipt: &str) -> bool {
        if self.transfer.inbox.view.loading
            || self.transfer.inbox.view.exporting
            || self.transfer.inbox.view.previewing
        {
            return false;
        }
        if !self
            .transfer
            .inbox
            .view
            .rows
            .iter()
            .any(|item| item.id == receipt && item.status_key == "transfer.inboxVerified")
        {
            return false;
        }
        let Ok(id) = Uuid::parse_str(receipt) else {
            return false;
        };
        let (tx, rx) = mpsc::channel();
        self.transfer.inbox.preview_rx = Some(rx);
        self.transfer.inbox.view.preview = Some((receipt.to_owned(), String::new()));
        self.transfer.inbox.view.previewing = true;
        self.transfer.inbox.view.notice_key = None;
        let wake = Arc::clone(&self.workspace.wake);
        thread::spawn(move || {
            let result = user_inbox_root()
                .map_err(|error| format!("inbox root: {error:?}"))
                .and_then(|root| read_checked_text_preview(&root, id));
            if tx.send(result).is_ok() {
                wake();
            }
        });
        true
    }

    /// A displayed UUID is only a selector, never a filename or permission.
    /// No stale, pending, tampered or duplicate-busy row may start Save Copy.
    pub(super) fn begin_inbox_save(&mut self, receipt: &str) -> bool {
        if self.transfer.inbox.view.exporting
            || self.transfer.inbox.view.loading
            || self.transfer.inbox.view.previewing
        {
            return false;
        }
        if !self
            .transfer
            .inbox
            .view
            .rows
            .iter()
            .any(|item| item.id == receipt && item.status_key == "transfer.inboxVerified")
        {
            return false;
        }
        let Ok(id) = Uuid::parse_str(receipt) else {
            return false;
        };
        // A user may select an arbitrary path in the system dialog. The
        // receiver must refuse protected application data and known Vaults:
        // that is a File Gate operation, not a manual Inbox export.
        let known = match self.workspace.core.ui_state.list_workspaces() {
            Ok(known) => known,
            Err(_) => {
                self.transfer.inbox.view.notice_key = Some("transfer.inboxSaveFailed");
                return true;
            }
        };
        let mut protected_roots = vec![self.workspace.data_dir.clone()];
        if let Some(root) = &self.workspace.root {
            protected_roots.push(root.clone());
        }
        protected_roots.extend(known.into_iter().map(|entry| PathBuf::from(entry.path)));
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
                    .and_then(|root| save_checked_copy(&root, id, &destination, &protected_roots))
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
    let Some(ledger) =
        InboxLedger::open_existing(root).map_err(|error| format!("read-only ledger: {error:?}"))?
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
        previewing: false,
        preview: None,
        notice_key: None,
    })
}

/// A deliberately narrow in-app text preview: only checked files up to 4 KiB.
/// Never execute remote bytes, launch handlers, use a sender path or write Vault.
fn read_checked_text_preview(root: &Path, id: Uuid) -> Result<Option<String>, String> {
    const MAX_TEXT_BYTES: u64 = 4 * 1024;
    let ledger = InboxLedger::open_existing(root)
        .map_err(|error| format!("open checked Inbox: {error:?}"))?
        .ok_or("Inbox does not exist")?;
    let (mut pinned, intent) = ledger
        .open_verified_file(id)
        .map_err(|error| format!("checked preview refused: {error:?}"))?;
    if intent.bytes > MAX_TEXT_BYTES {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(intent.bytes as usize);
    pinned.read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() as u64 != intent.bytes
        || pinned.metadata().map_err(|error| error.to_string())?.len() != intent.bytes
        || *blake3::hash(&bytes).as_bytes() != intent.blake3
    {
        return Err("Inbox file changed during preview".into());
    }
    if bytes.contains(&0) {
        return Ok(None);
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return Ok(None);
    };
    if text.chars().any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t')) {
        return Ok(None);
    }
    Ok(Some(text))
}

fn save_checked_copy(
    root: &Path,
    id: Uuid,
    destination: &Path,
    protected_roots: &[PathBuf],
) -> Result<(), String> {
    if !destination.is_absolute() {
        return Err("destination must be an absolute new path".into());
    }
    let parent = destination.parent().ok_or("missing destination parent")?;
    let canonical_parent =
        fs::canonicalize(parent).map_err(|error| format!("destination parent: {error}"))?;
    // This is a best-effort fixed-root gate, not a proof against hostile
    // ancestor substitution after this check. Release requires handle custody.
    for protected in std::iter::once(root).chain(protected_roots.iter().map(PathBuf::as_path)) {
        if let Ok(canonical) = fs::canonicalize(protected) {
            if canonical_parent.starts_with(&canonical) {
                return Err("Save copy into Akula data or a known Vault is forbidden".into());
            }
        }
    }
    let Some(ledger) =
        InboxLedger::open_existing(root).map_err(|error| format!("read-only ledger: {error:?}"))?
    else {
        return Err("receiver inbox does not exist".into());
    };
    // Recheck the complete same opened file before attempting a new destination.
    let (mut pinned, intent) = ledger
        .open_verified_file(id)
        .map_err(|error| format!("received object invalid: {error:?}"))?;
    save_verified_stage_as_new(&mut pinned, intent.bytes, intent.blake3, destination)
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
        let inbox_root = dir.path().join("received-inbox");
        let mut ledger = InboxLedger::open(&inbox_root).unwrap();
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
        let model = read_checked_inbox(&inbox_root).unwrap();
        assert_eq!(model.total, 1);
        assert_eq!(model.rows[0].status_key, "transfer.inboxVerified");
        assert_eq!(
            read_checked_text_preview(&inbox_root, file.id).unwrap().as_deref(),
            Some("read-only native UI physical receipt")
        );

        let export_dir = dir.path().join("external-exports");
        fs::create_dir(&export_dir).unwrap();
        let copy = export_dir.join("new-copy.bin");
        save_checked_copy(&inbox_root, file.id, &copy, &[]).unwrap();
        assert_eq!(fs::read(&copy).unwrap(), bytes);
        // No replacing even a user-created or previously exported file.
        assert!(save_checked_copy(&inbox_root, file.id, &copy, &[]).is_err());
        assert_eq!(fs::read(&copy).unwrap(), bytes);

        fs::write(&path, b"modified").unwrap();
        let model = read_checked_inbox(&inbox_root).unwrap();
        assert_eq!(model.total, 1);
        assert_eq!(model.rows[0].status_key, "transfer.inboxReview");
        assert!(read_checked_text_preview(&inbox_root, file.id).is_err());
        let refused = export_dir.join("never-created.bin");
        assert!(save_checked_copy(&inbox_root, file.id, &refused, &[]).is_err());
        assert!(!refused.exists());
    }

    #[test]
    fn save_copy_rejects_own_inbox_and_known_vault() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("my-vault");
        fs::create_dir(&vault).unwrap();
        let bytes = b"the user must explicitly leave the protected Vault";
        let intent = ExpectedTransfer {
            id: Uuid::new_v4(),
            sender_key: [51; 32],
            recipient_key: [52; 32],
            source: Transport::P2p,
            bytes: bytes.len() as u64,
            blake3: *blake3::hash(bytes).as_bytes(),
        };
        let inbox = dir.path().join("received-inbox");
        let mut ledger = InboxLedger::open(&inbox).unwrap();
        ledger.reserve(&intent).unwrap();
        let path = ledger.object_path(intent.id);
        let mut writer = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        writer.write_all(bytes).unwrap();
        writer.sync_all().unwrap();
        drop(writer);
        ledger.confirm(&intent).unwrap();
        drop(ledger);
        let forbidden_vault = vault.join("unexpected.txt");
        assert!(save_checked_copy(&inbox, intent.id, &forbidden_vault, &[vault.clone()]).is_err());
        assert!(!forbidden_vault.exists());
        let forbidden_inbox = inbox.join("unexpected.txt");
        assert!(save_checked_copy(&inbox, intent.id, &forbidden_inbox, &[]).is_err());
        assert!(!forbidden_inbox.exists());
    }
}
