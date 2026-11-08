//! Explicit Windows-only, user-selected Inbox -> registered Vault import.
//! The standard signed Windows feature graph does not include this UI.
use super::*;
use aquilum_core::{Core, files::gate::import_verified_inbox_as_new};

impl InboxPreview {
    pub(super) fn poll_import_result(&mut self, changed: &mut bool) {
        let outcome = self.import_rx.as_ref().map(|receiver| receiver.try_recv());
        match outcome {
            Some(Ok(Ok(Some(receipt)))) => {
                if let Some(row) = self
                    .view
                    .rows
                    .iter_mut()
                    .find(|row| row.id == receipt.hyphenated().to_string())
                {
                    row.import_audit_key = Some("transfer.inboxAuditVerified");
                }
                self.view.importing = false;
                self.view.notice_key = Some("transfer.inboxImported");
                self.import_rx = None;
                *changed = true;
            }
            Some(Ok(Ok(None))) => {
                self.view.importing = false;
                self.import_rx = None;
                *changed = true;
            }
            Some(Ok(Err(_))) | Some(Err(TryRecvError::Disconnected)) => {
                self.view.importing = false;
                self.view.notice_key = Some("transfer.inboxImportFailed");
                self.import_rx = None;
                *changed = true;
            }
            _ => {}
        }
    }
}

impl App {
    pub(in crate::app::transfer) fn begin_inbox_import(&mut self, receipt: &str) -> bool {
        if self.transfer.inbox.view.loading
            || self.transfer.inbox.view.exporting
            || self.transfer.inbox.view.importing
            || self.transfer.inbox.view.previewing
        {
            return false;
        }
        if !self.transfer.inbox.view.rows.iter().any(|row| {
            row.id == receipt
                && row.status_key == "transfer.inboxVerified"
                && row.import_audit_key.is_none()
        }) {
            return false;
        }
        let Ok(id) = Uuid::parse_str(receipt) else {
            return false;
        };
        let (tx, rx) = mpsc::channel();
        self.transfer.inbox.import_rx = Some(rx);
        self.transfer.inbox.view.importing = true;
        self.transfer.inbox.view.notice_key = None;
        let wake = Arc::clone(&self.workspace.wake);
        let core = Arc::clone(&self.workspace.core);
        thread::spawn(move || {
            // The user must select a registered Vault root explicitly. This
            // folder chooser cannot grant writes to an arbitrary directory.
            let chosen = rfd::FileDialog::new().pick_folder();
            let result = match chosen {
                None => Ok(None),
                Some(vault_root) => user_inbox_root()
                    .map_err(|error| format!("Inbox root: {error:?}"))
                    .and_then(|root| import_checked_into_vault(&core, &root, id, &vault_root))
                    .map(|_| Some(id)),
            };
            if tx.send(result).is_ok() {
                wake();
            }
        });
        true
    }
}

/// Reopen an immutable verified local ledger receipt at the moment of import.
/// Neither a stale display row nor a network-provided filename is authority.
fn import_checked_into_vault(
    core: &Core,
    root: &Path,
    receipt: Uuid,
    vault_root: &Path,
) -> Result<PathBuf, String> {
    let _pin = aquilum_core::p2p_inbox_lab::pin_windows_inbox_read(root)
        .map_err(|error| format!("Inbox custody: {error:?}"))?;
    let ledger = InboxLedger::open_existing(root)
        .map_err(|error| format!("Inbox ledger: {error:?}"))?
        .ok_or("Inbox ledger missing")?;
    let imported = ledger
        .import_status(receipt)
        .map_err(|error| format!("Inbox import audit unavailable: {error:?}"))?;
    if imported != akula_inbox_ledger_lab::ImportAuditStatus::NotRecorded {
        return Err("Previous import is recorded or requires manual review".into());
    }
    let (mut verified, expected) = ledger
        .open_verified_file(receipt)
        .map_err(|error| format!("Inbox file changed: {error:?}"))?;
    if expected.id != receipt {
        return Err("Inbox receipt identity mismatch".into());
    }
    drop(ledger);
    let destination = import_verified_inbox_as_new(
        core,
        root,
        vault_root,
        &mut verified,
        expected.bytes,
        expected.blake3,
        expected.id,
    )
    .map_err(|error| format!("Vault File Gate import refused: {error}"))?;
    let audit = InboxLedger::open(root)
        .map_err(|error| format!("Vault file may exist; Inbox audit unavailable: {error:?}"))?;
    audit
        .record_imported(receipt, &destination)
        .map_err(|error| {
            format!("Vault file may exist; import requires manual review: {error:?}")
        })?;
    Ok(destination)
}

#[cfg(test)]
mod tests;
