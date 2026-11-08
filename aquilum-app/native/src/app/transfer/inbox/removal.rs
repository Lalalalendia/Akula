//! Durable Inbox *intent* only. Never unlink/replace files from this QA path.
//! Physical Win7 same-HANDLE disposition remains a separate unshipped gate.
use super::*;

impl InboxPreview {
    pub(super) fn poll_removal_result(&mut self, changed: &mut bool) {
        let outcome = self.removal_rx.as_ref().map(|receiver| receiver.try_recv());
        match outcome {
            Some(Ok(Ok(id))) => {
                if let Some(row) = self.view.rows.iter_mut().find(|row| row.id == id.to_string()) {
                    row.status_key = "transfer.inboxRemovalRequested";
                }
                self.view.removing = false;
                self.view.preview = None;
                self.view.removal_confirmation = None;
                self.view.notice_key = Some("transfer.inboxRemoveSaved");
                self.removal_rx = None;
                *changed = true;
            }
            Some(Ok(Err(_))) | Some(Err(TryRecvError::Disconnected)) => {
                self.view.removing = false;
                self.view.removal_confirmation = None;
                self.view.notice_key = Some("transfer.inboxRemoveFailed");
                self.removal_rx = None;
                *changed = true;
            }
            _ => {}
        }
    }
}

impl App {
    pub(in crate::app::transfer) fn ask_inbox_removal(&mut self, receipt: &str) -> bool {
        let view = &mut self.transfer.inbox.view;
        if view.loading || view.exporting || view.importing || view.previewing
            || view.removing || view.removal_confirmation.is_some()
        {
            return false;
        }
        if !view.rows.iter().any(|row| {
            row.id == receipt && row.status_key == "transfer.inboxVerified"
        }) || Uuid::parse_str(receipt).is_err()
        {
            return false;
        }
        view.removal_confirmation = Some(receipt.to_owned());
        view.notice_key = None;
        true
    }

    pub(in crate::app::transfer) fn cancel_inbox_removal(&mut self) -> bool {
        self.transfer.inbox.view.removal_confirmation.take().is_some()
    }

    pub(in crate::app::transfer) fn begin_inbox_removal(&mut self, receipt: &str) -> bool {
        let view = &mut self.transfer.inbox.view;
        if view.loading || view.exporting || view.importing || view.previewing || view.removing
            || view.removal_confirmation.as_deref() != Some(receipt)
            || !view.rows.iter().any(|row| {
                row.id == receipt && row.status_key == "transfer.inboxVerified"
            })
        {
            return false;
        }
        let Ok(id) = Uuid::parse_str(receipt) else {
            return false;
        };
        let (tx, rx) = mpsc::channel();
        self.transfer.inbox.removal_rx = Some(rx);
        view.removing = true;
        view.removal_confirmation = None;
        view.notice_key = None;
        let wake = Arc::clone(&self.workspace.wake);
        thread::spawn(move || {
            let result = user_inbox_root()
                .map_err(|error| format!("Inbox root: {error:?}"))
                .and_then(|root| request_checked_removal(&root, id))
                .map(|()| id);
            if tx.send(result).is_ok() {
                wake();
            }
        });
        true
    }
}

/// Independently recheck a locally verified object before committing SQLite intent.
/// This is deliberately non-destructive: even success keeps the original bytes.
fn request_checked_removal(root: &Path, id: Uuid) -> Result<(), String> {
    if id.is_nil() {
        return Err("Invalid Inbox receipt".into());
    }
    #[cfg(windows)]
    let _pinned = aquilum_core::p2p_inbox_lab::pin_windows_inbox_read(root)
        .map_err(|error| format!("Inbox custody: {error:?}"))?;
    let reader = InboxLedger::open_existing(root)
        .map_err(|error| format!("Inbox ledger: {error:?}"))?
        .ok_or("Inbox missing")?;
    let (verified, expected) = reader.open_verified_file(id)
        .map_err(|error| format!("Inbox receipt not available: {error:?}"))?;
    if expected.id != id {
        return Err("Inbox receipt identity mismatch".into());
    }
    drop(verified);
    drop(reader);
    let ledger = InboxLedger::open(root)
        .map_err(|error| format!("Inbox write ledger: {error:?}"))?;
    ledger.request_removal(id)
        .map_err(|error| format!("Inbox removal intent refused: {error:?}"))?;
    Ok(())
}
