//! Native read-side projection and explicit Save Copy from checked Inbox.
#[cfg(all(feature = "installed", feature = "p2p-inbox-physical-delete-qa"))]
compile_error!("QA physical Inbox deletion cannot be combined with installed");
#[cfg(all(feature = "p2p-inbox-physical-delete-qa", not(debug_assertions)))]
compile_error!("QA physical Inbox deletion cannot ship in a release build");
//! No automatic network transport, sender ACK, Vault writes or overwrite.
//! An export rehashes the same pinned file descriptor at click time.

use super::super::App;
use crate::ui::transfer::InboxView;
use akula_inbox_ledger_lab::{InboxLedger, user_inbox_root};
use aquilum_core::p2p_inbox_lab::save_verified_stage_as_new;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
};
use uuid::Uuid;
#[cfg(windows)]
mod import;
mod list;
mod preview;
mod removal;
#[cfg(all(windows, feature = "p2p-inbox-physical-delete-qa"))]
mod physical_removal;
use list::{PAGE_SIZE, read_checked_inbox_page};
use preview::read_checked_text_preview;

#[derive(Default)]
pub(super) struct InboxPreview {
    pub(super) view: InboxView,
    rx: Option<Receiver<Result<InboxView, String>>>,
    save_rx: Option<Receiver<Result<bool, String>>>,
    #[cfg(windows)]
    import_rx: Option<Receiver<Result<Option<Uuid>, String>>>,
    #[cfg(windows)]
    pub(super) refresh_vault_copies: bool,
    preview_rx: Option<Receiver<Result<Option<String>, String>>>,
    removal_rx: Option<Receiver<Result<Uuid, String>>>,
    #[cfg(all(windows, feature = "p2p-inbox-physical-delete-qa"))]
    physical_removal_rx: Option<Receiver<Result<Uuid, String>>>,
    #[cfg(all(windows, feature = "p2p-inbox-physical-delete-qa"))]
    physical_removal_pending: Option<Uuid>,
}

impl InboxPreview {
    fn begin(&mut self, receiver: Receiver<Result<InboxView, String>>) {
        self.rx = Some(receiver);
        self.preview_rx = None;
        self.view.loading = true;
        self.view.previewing = false;
        self.view.preview = None;
        self.view.removal_confirmation = None;
        #[cfg(all(windows, feature = "p2p-inbox-physical-delete-qa"))]
        {
            self.view.physical_removal_confirmation = None;
        }
        self.view.error = None;
    }

    pub(super) fn poll(&mut self) -> bool {
        let mut changed = false;
        let next = self.rx.as_ref().map(|receiver| receiver.try_recv());
        match next {
            Some(Ok(Ok(mut view))) => {
                view.exporting = self.view.exporting;
                view.importing = self.view.importing;
                view.previewing = self.view.previewing;
                view.preview = self.view.preview.clone();
                view.notice_key = self.view.notice_key;
                view.removing = self.view.removing;
                view.removal_confirmation = self.view.removal_confirmation.clone();
                #[cfg(all(windows, feature = "p2p-inbox-physical-delete-qa"))]
                {
                    view.physical_removal_confirmation =
                        self.view.physical_removal_confirmation.clone();
                }
                self.view = view;
                self.rx = None;
                changed = true;
            }
            Some(Ok(Err(error))) => {
                self.view = InboxView {
                    error: Some(error),
                    exporting: self.view.exporting,
                    importing: self.view.importing,
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
        #[cfg(windows)]
        self.poll_import_result(&mut changed);
        self.poll_removal_result(&mut changed);
        #[cfg(all(windows, feature = "p2p-inbox-physical-delete-qa"))]
        self.poll_physical_removal_result(&mut changed);
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
        let offset = self.transfer.inbox.view.page_offset;
        let (tx, rx) = mpsc::channel();
        self.transfer.inbox.begin(rx);
        let wake = Arc::clone(&self.workspace.wake);
        thread::spawn(move || {
            let result = user_inbox_root()
                .map_err(|err| format!("inbox root: {err:?}"))
                .and_then(|root| read_checked_inbox_page(&root, offset));
            let _ = tx.send(result);
            wake();
        });
    }

    pub(super) fn change_inbox_page(&mut self, older: bool) -> bool {
        let view = &self.transfer.inbox.view;
        if view.loading
            || view.exporting
            || view.importing
            || view.previewing
            || view.removing
            || view.removal_confirmation.is_some()
            || view.has_physical_confirmation()
        {
            return false;
        }
        let next = if older {
            view.page_offset.checked_add(PAGE_SIZE)
        } else {
            view.page_offset.checked_sub(PAGE_SIZE)
        };
        let Some(next) = next else {
            return false;
        };
        if older && u64::try_from(next).map_or(true, |offset| offset >= view.total) {
            return false;
        }
        self.transfer.inbox.view.page_offset = next;
        self.begin_inbox_preview();
        true
    }

    /// The preview is user-triggered. A displayed UUID is only a selector,
    /// never a filename, path or authorization to read arbitrary files.
    pub(super) fn begin_inbox_text_preview(&mut self, receipt: &str) -> bool {
        if self.transfer.inbox.view.loading
            || self.transfer.inbox.view.exporting
            || self.transfer.inbox.view.importing
            || self.transfer.inbox.view.previewing
            || self.transfer.inbox.view.removing
            || self.transfer.inbox.view.removal_confirmation.is_some()
            || self.transfer.inbox.view.has_physical_confirmation()
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
            || self.transfer.inbox.view.importing
            || self.transfer.inbox.view.loading
            || self.transfer.inbox.view.previewing
            || self.transfer.inbox.view.removing
            || self.transfer.inbox.view.removal_confirmation.is_some()
            || self.transfer.inbox.view.has_physical_confirmation()
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
    #[cfg(windows)]
    let _pinned_export_dir = aquilum_core::p2p_inbox_lab::pin_windows_export_parent(parent)
        .map_err(|error| format!("destination parent custody: {error:?}"))?;
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
    #[cfg(windows)]
    let _inbox_guard = aquilum_core::p2p_inbox_lab::pin_windows_inbox_read(root)
        .map_err(|error| format!("Windows Inbox export custody refused: {error:?}"))?;
    // Recheck the complete same opened file before attempting a new destination.
    let (mut pinned, intent) = ledger
        .open_verified_file(id)
        .map_err(|error| format!("received object invalid: {error:?}"))?;
    save_verified_stage_as_new(&mut pinned, intent.bytes, intent.blake3, destination)
        .map_err(|error| format!("Save Copy refused: {error:?}"))?;
    Ok(())
}

#[cfg(test)]
mod tests;
