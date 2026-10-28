//! Explicit Windows-only, user-selected Inbox -> registered Vault import.
//! The standard signed Windows feature graph does not include this UI.
use super::*;
use aquilum_core::{Core, files::gate::import_verified_inbox_as_new};

impl InboxPreview {
    pub(super) fn poll_import_result(&mut self, changed: &mut bool) {
        let outcome = self.import_rx.as_ref().map(|receiver| receiver.try_recv());
        match outcome {
            Some(Ok(Ok(true))) => {
                self.view.importing = false;
                self.view.notice_key = Some("transfer.inboxImported");
                self.import_rx = None;
                *changed = true;
            }
            Some(Ok(Ok(false))) => {
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
    pub(super) fn begin_inbox_import(&mut self, receipt: &str) -> bool {
        if self.transfer.inbox.view.loading
            || self.transfer.inbox.view.exporting
            || self.transfer.inbox.view.importing
            || self.transfer.inbox.view.previewing
        {
            return false;
        }
        if !self.transfer.inbox.view.rows.iter().any(|row| {
            row.id == receipt && row.status_key == "transfer.inboxVerified"
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
                None => Ok(false),
                Some(vault_root) => user_inbox_root()
                    .map_err(|error| format!("Inbox root: {error:?}"))
                    .and_then(|root| import_checked_into_vault(&core, &root, id, &vault_root))
                    .map(|_| true),
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
    let (mut verified, expected) = ledger
        .open_verified_file(receipt)
        .map_err(|error| format!("Inbox file changed: {error:?}"))?;
    if expected.id != receipt {
        return Err("Inbox receipt identity mismatch".into());
    }
    import_verified_inbox_as_new(
        core,
        root,
        vault_root,
        &mut verified,
        expected.bytes,
        expected.blake3,
        expected.id,
    )
    .map_err(|error| format!("Vault File Gate import refused: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use akula_inbox_ledger_lab::{ExpectedTransfer, LocalStatus, Transport};
    use aquilum_core::CoreEvent;
    use std::{fs, io::Write};

    #[test]
    fn verified_receipt_import_creates_new_file_only() {
        let temp = tempfile::tempdir().unwrap();
        let appdata = temp.path().join("appdata");
        let inbox = temp.path().join("received-inbox");
        let vault = temp.path().join("vault");
        for path in [&appdata, &vault] {
            fs::create_dir(path).unwrap();
        }
        let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(vault.to_str().unwrap(), 1).unwrap();
        let mut ledger = InboxLedger::open(&inbox).unwrap();
        let bytes = b"verified inbox content";
        let receipt = ExpectedTransfer {
            id: Uuid::new_v4(),
            sender_key: [7; 32],
            recipient_key: [8; 32],
            source: Transport::P2p,
            bytes: bytes.len() as u64,
            blake3: *blake3::hash(bytes).as_bytes(),
        };
        assert_eq!(ledger.reserve(&receipt).unwrap(), LocalStatus::Pending);
        let object = ledger.object_path(receipt.id);
        let mut writer = fs::OpenOptions::new().create_new(true).write(true).open(&object).unwrap();
        writer.write_all(bytes).unwrap();
        writer.sync_all().unwrap();
        drop(writer);
        assert_eq!(ledger.confirm(&receipt).unwrap(), LocalStatus::VerifiedLocal);
        drop(ledger);
        let imported = import_checked_into_vault(&core, &inbox, receipt.id, &vault).unwrap();
        assert_eq!(fs::read(&imported).unwrap(), bytes);
        assert_eq!(fs::read(&object).unwrap(), bytes);
        assert!(import_checked_into_vault(&core, &inbox, receipt.id, &vault).is_err());
        assert_eq!(fs::read(&imported).unwrap(), bytes);
        assert_eq!(fs::read(&object).unwrap(), bytes);
        fs::write(&object, b"corrupted").unwrap();
        assert!(import_checked_into_vault(&core, &inbox, receipt.id, &vault).is_err());
    }
}
