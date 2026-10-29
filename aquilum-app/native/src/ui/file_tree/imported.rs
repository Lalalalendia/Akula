//! QA-only, read-only projection of locally verified P2P Vault copies.
//! File Gate permissions and ordinary note-tree file types are unchanged.
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
};

use akula_inbox_ledger_lab::{InboxLedger, user_inbox_root};
use masonry::core::{NewWidget, PropertySet};
use masonry::layout::Length;
use masonry::properties::types::CrossAxisAlignment;
use masonry::properties::{Gap, Padding};
use masonry::widgets::Flex;

use super::FileTree;
use crate::i18n::t;
use crate::ui::tokens::size;
use crate::ui::widgets::Label;

type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
pub(super) struct ImportedScan {
    names: Vec<String>,
    receiver: Option<Receiver<(PathBuf, Vec<String>)>>,
    pending: bool,
}

impl ImportedScan {
    pub(super) fn names(&self) -> &[String] {
        &self.names
    }

    pub(super) fn request(&mut self, vault: PathBuf, wake: Wake) {
        if self.receiver.is_some() {
            self.pending = true;
        } else {
            self.start(vault, wake);
        }
    }

    fn start(&mut self, vault: PathBuf, wake: Wake) {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let names = read_verified_names(&vault);
            if sender.send((vault, names)).is_ok() {
                wake();
            }
        });
        self.receiver = Some(receiver);
    }

    pub(super) fn poll(&mut self, vault: &Path, wake: Wake) -> bool {
        let status = self.receiver.as_ref().map(Receiver::try_recv);
        match status {
            Some(Ok((scanned_vault, names))) => {
                self.receiver = None;
                if self.pending || scanned_vault != vault {
                    self.pending = false;
                    let changed = !self.names.is_empty();
                    self.names.clear();
                    self.start(vault.to_path_buf(), wake);
                    return changed;
                }
                let changed = self.names != names;
                self.names = names;
                changed
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.receiver = None;
                self.pending = false;
                let changed = !self.names.is_empty();
                self.names.clear();
                changed
            }
            _ => false,
        }
    }
}

impl FileTree {
    pub(crate) fn request_imported(&mut self) {
        if let Some(vault) = self.model.root() {
            self.imported
                .request(vault.to_path_buf(), Arc::clone(&self.wake));
        }
    }

    pub(super) fn poll_imported(&mut self) -> bool {
        let Some(vault) = self.model.root() else {
            return false;
        };
        self.imported.poll(vault, Arc::clone(&self.wake))
    }
}

fn verified_names(vault: &Path, inbox: &Path) -> Vec<String> {
    let Ok(Some(ledger)) = InboxLedger::open_existing(inbox) else {
        return Vec::new();
    };
    ledger
        .verified_imports_in(vault)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|path| path.file_name()?.to_str().map(str::to_owned))
        .collect()
}

/// All SQLite access, rehashing, and Windows handle pinning run off the UI thread.
fn read_verified_names(vault: &Path) -> Vec<String> {
    let Ok(inbox) = user_inbox_root() else {
        return Vec::new();
    };
    let Ok(_pinned) = aquilum_core::p2p_inbox_lab::pin_windows_inbox_read(&inbox) else {
        return Vec::new();
    };
    verified_names(vault, &inbox)
}

/// Noninteractive names only: no file open, rename, move, or delete action.
pub(super) fn render(names: &[String]) -> Option<NewWidget<Flex>> {
    if names.is_empty() {
        return None;
    }
    let mut panel = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(NewWidget::new(Label::new(
            &t("transfer.inboxVaultCopies"),
            |t| t.text_secondary,
        )));
    for name in names {
        panel = panel.with_fixed(NewWidget::new(Label::new(name, |t| t.text_secondary)));
    }
    panel = panel.with_fixed(NewWidget::new(Label::new(
        &t("transfer.inboxVaultCopiesHint"),
        |t| t.text_tertiary,
    )));
    Some(
        NewWidget::new(panel).with_props(
            PropertySet::new()
                .with(Gap::new(Length::px(size::SPACE_4)))
                .with(Padding::all(Length::px(size::SPACE_8))),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use akula_inbox_ledger_lab::{ExpectedTransfer, Transport};
    use std::{fs, io::Write};
    use uuid::Uuid;

    #[test]
    fn only_recorded_reverified_vault_copy_is_listed() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = dir.path().join("inbox");
        let vault = dir.path().join("vault");
        let other = dir.path().join("other");
        fs::create_dir(&vault).unwrap();
        fs::create_dir(&other).unwrap();
        let bytes = b"checked local p2p import";
        let intent = ExpectedTransfer {
            id: Uuid::new_v4(),
            sender_key: [11; 32],
            recipient_key: [12; 32],
            source: Transport::P2p,
            bytes: bytes.len() as u64,
            blake3: *blake3::hash(bytes).as_bytes(),
        };
        let mut ledger = InboxLedger::open(&inbox).unwrap();
        ledger.reserve(&intent).unwrap();
        let original = ledger.object_path(intent.id);
        let mut writer = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&original)
            .unwrap();
        writer.write_all(bytes).unwrap();
        writer.sync_all().unwrap();
        drop(writer);
        ledger.confirm(&intent).unwrap();

        let name = format!("received-{}.bin", intent.id);
        let target = vault.join(&name);
        fs::write(&target, bytes).unwrap();
        assert!(verified_names(&vault, &inbox).is_empty());
        ledger.record_imported(intent.id, &target).unwrap();
        assert_eq!(verified_names(&vault, &inbox), vec![name]);
        assert!(verified_names(&other, &inbox).is_empty());
        fs::write(&target, b"modified after import").unwrap();
        assert!(verified_names(&vault, &inbox).is_empty());
        assert_eq!(fs::read(&original).unwrap(), bytes);
    }

    #[test]
    fn old_worker_result_cannot_replace_a_different_vault() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let mut scan = ImportedScan {
            names: vec!["old.bin".into()],
            receiver: Some(rx),
            pending: false,
        };
        tx.send((first.path().to_path_buf(), vec!["stale.bin".into()]))
            .unwrap();
        assert!(scan.poll(second.path(), Arc::new(|| {})));
        assert!(scan.names().is_empty());
    }
}
