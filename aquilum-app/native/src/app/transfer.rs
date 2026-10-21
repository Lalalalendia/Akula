//! Local transfer drafts only. The network service, peer discovery, File Gate,
//! delivery receipts and verified progress remain unavailable in this UI.

use std::path::PathBuf;
use std::sync::Arc;

use aquilum_core::sync::DeviceId;
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};
use masonry::kurbo::Point;

use super::{App, Outcome, Popup};
use crate::i18n::t;
use crate::ui::dialog::{self, DialogAnswer};
use crate::ui::transfer::{self as transfer_ui, QueueAction};
use crate::ui::widgets::Pressed;

const MAX_DRAFTS: usize = 32;
const MAX_DEVICE_ROWS: usize = 32;

/// A display-only identity observation. The first-party owner registry still
/// needs to supply a verified snapshot. No production path populates this
/// collection, and neither this struct nor the UI authorizes File Gate writes.
#[derive(Clone, Debug)]
pub(super) struct TransferPeer {
    pub(super) id: DeviceId,
    pub(super) fingerprint: [u8; 32],
    pub(super) name: String,
    pub(super) status: transfer_ui::DeviceStatus,
}

#[derive(Clone, Debug)]
pub(super) struct Draft {
    pub(super) path: PathBuf,
    pub(super) bytes: u64,
    pub(super) paused: bool,
}

#[derive(Default)]
pub(super) struct TransferState {
    pub(super) drafts: Vec<Draft>,
    pub(super) choosing: bool,
    pub(super) error: Option<String>,
    peers: Vec<TransferPeer>,
    pub(super) selected_peer: Option<(DeviceId, [u8; 32])>,
}

impl TransferState {
    pub(super) fn stage(&mut self, paths: Vec<PathBuf>) {
        self.error = None;
        for path in paths {
            if self.drafts.iter().any(|entry| entry.path == path) {
                continue;
            }
            if self.drafts.len() >= MAX_DRAFTS {
                self.error = Some(t("transfer.limit"));
                break;
            }
            // Metadata only. Never read bytes, compute a digest, write a file,
            // make a network call, follow symlinks or claim a verified transfer.
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                self.error = Some(t("transfer.invalidFile"));
                continue;
            };
            if !metadata.file_type().is_file() || path.file_name().is_none() {
                self.error = Some(t("transfer.invalidFile"));
                continue;
            }
            self.drafts.push(Draft { path, bytes: metadata.len(), paused: false });
        }
    }

    pub(super) fn toggle(&mut self, index: usize) {
        if let Some(item) = self.drafts.get_mut(index) {
            item.paused = !item.paused;
        }
    }

    pub(super) fn remove(&mut self, index: usize) {
        if index < self.drafts.len() {
            self.drafts.remove(index);
        }
    }

    pub(super) fn choose_peer(&mut self, binding: (DeviceId, [u8; 32])) {
        // Always match the *exact* displayed DeviceId and key, never a stale
        // list index which might now refer to a completely different peer.
        let matches: Vec<_> = self.peers.iter().filter(|peer| peer.id == binding.0).collect();
        self.selected_peer = if matches.len() == 1
            && matches[0].fingerprint == binding.1
            && binding.1 != [0; 32]
            && matches[0].status == transfer_ui::DeviceStatus::IdentityChecked
        {
            Some(binding)
        } else {
            None
        };
    }

    fn reconcile_selection(&mut self) {
        if let Some(binding) = self.selected_peer {
            let matches: Vec<_> = self.peers.iter().filter(|peer| peer.id == binding.0).collect();
            if matches.len() != 1 || matches[0].fingerprint != binding.1
                || matches[0].fingerprint == [0; 32]
                || matches[0].status != transfer_ui::DeviceStatus::IdentityChecked
            {
                self.selected_peer = None;
            }
        }
    }

    /// Synthetic source seam, compiled only in tests. In product builds a
    /// signed, custody-checked first-party registry adapter is a prerequisite.
    #[cfg(test)]
    pub(super) fn apply_test_peer_snapshot(&mut self, peers: Vec<TransferPeer>) {
        // Refuse oversize rosters rather than truncating away a conflicting ID.
        if peers.len() > MAX_DEVICE_ROWS {
            self.peers.clear();
            self.selected_peer = None;
            return;
        }
        self.peers = peers.into_iter()
            .filter(|peer| !peer.id.as_uuid().is_nil())
            .collect();
        // Any duplicate DeviceId is ambiguous, even with identical key bytes;
        // no UI selection can authorize a transfer in any case.
        let mut ambiguous = Vec::new();
        for (index, peer) in self.peers.iter().enumerate() {
            if self.peers.iter().enumerate().any(|(other_index, other)| {
                other_index != index && other.id == peer.id
            }) {
                ambiguous.push(index);
            }
        }
        for index in ambiguous {
            self.peers[index].status = transfer_ui::DeviceStatus::Unverified;
        }
        self.reconcile_selection();
    }

    pub(super) fn device_rows(&self) -> Vec<transfer_ui::DeviceRow> {
        self.peers.iter().take(MAX_DEVICE_ROWS).map(|peer| {
            let name: String = peer.name.chars()
                .filter(|c| !c.is_control()).take(64).collect();
            let name = if name.trim().is_empty() {
                peer.id.to_string()
            } else {
                name
            };
            // Public-key fingerprints are display metadata, not evidence that
            // a device is online, allowed to read, or entitled to receive.
            let fingerprint = peer.fingerprint[..8].iter()
                .map(|byte| format!("{byte:02x}")).collect::<String>();
            transfer_ui::DeviceRow {
                id: peer.id, key: peer.fingerprint, name, fingerprint,
                status: if peer.fingerprint == [0; 32] {
                    transfer_ui::DeviceStatus::Unverified
                } else {
                    peer.status
                },
                selected: self.selected_peer == Some((peer.id, peer.fingerprint)),
            }
        }).collect()
    }

    fn rows(&self) -> Vec<transfer_ui::QueueRow> {
        self.drafts.iter().enumerate().map(|(index, item)| {
            let raw = item.path.file_name().unwrap_or_default().to_string_lossy();
            let mut name: String = raw.chars().filter(|c| !c.is_control()).take(96).collect();
            if raw.chars().count() > 96 {
                name.push('…');
            }
            transfer_ui::QueueRow { index, name, bytes: item.bytes, paused: item.paused }
        }).collect()
    }
}

impl App {
    pub(super) fn open_transfers(&mut self, root: &mut RenderRoot) {
        if self.popup.is_some() {
            return;
        }
        self.show_transfers(root);
    }

    fn show_transfers(&mut self, root: &mut RenderRoot) {
        let (layer, dialog) = transfer_ui::dialog(
            &self.transfer.rows(), &self.transfer.device_rows(),
            self.transfer.choosing, self.transfer.error.as_deref(),
        );
        root.add_layer(layer, Point::ORIGIN);
        root.focus_on(Some(dialog.layer));
        self.popup = Some(Popup::Transfers(dialog));
    }

    fn refresh_transfers(&mut self, root: &mut RenderRoot) {
        if let Some(Popup::Transfers(dialog)) = self.popup.take() {
            root.remove_layer(dialog.layer);
            self.show_transfers(root);
        }
    }

    pub(super) fn on_transfer_picked(&mut self, root: &mut RenderRoot) {
        let mut changed = false;
        while let Ok(paths) = self.transfer_rx.try_recv() {
            self.transfer.choosing = false;
            self.transfer.stage(paths);
            changed = true;
        }
        if changed {
            self.refresh_transfers(root);
        }
    }

    fn choose_transfer_files(&mut self) {
        if self.transfer.choosing {
            return;
        }
        self.transfer.choosing = true;
        let tx = self.transfer_tx.clone();
        let wake = Arc::clone(&self.workspace.wake);
        std::thread::spawn(move || {
            let picked = rfd::FileDialog::new().pick_files().unwrap_or_default();
            if tx.send(picked).is_ok() {
                wake();
            }
        });
    }

    pub(super) fn on_transfer_action(&mut self, root: &mut RenderRoot, id: WidgetId, action: &ErasedAction) -> Outcome {
        let Some(Popup::Transfers(dialog)) = self.popup.take() else { return Outcome::default() };
        let pressed = action.downcast_ref::<Pressed>().is_some();
        let dismiss = (id == dialog.layer && matches!(action.downcast_ref::<DialogAnswer>(), Some(DialogAnswer::Cancel)))
            || (id == dialog.close && pressed);
        if dismiss {
            dialog::close(root, dialog.layer);
            return Outcome::default();
        }
        let mut refresh = false;
        if id == dialog.choose && pressed {
            self.choose_transfer_files();
            refresh = true;
        } else if pressed
            && let Some((_, index, command)) = dialog.actions.iter().find(|(button, _, _)| *button == id)
        {
            match command {
                QueueAction::Toggle => self.transfer.toggle(*index),
                QueueAction::Remove => self.transfer.remove(*index),
            }
            refresh = true;
        } else if pressed
            && let Some((_, device_id, key)) = dialog.device_actions.iter().find(|(button, _, _)| *button == id)
        {
            self.transfer.choose_peer((*device_id, *key));
            refresh = true;
        }
        self.popup = Some(Popup::Transfers(dialog));
        if refresh {
            self.refresh_transfers(root);
        }
        Outcome::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drafts_are_metadata_only_and_cannot_start_transfer() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("archive.bin");
        std::fs::write(&file, b"hello").unwrap();
        let mut queue = TransferState::default();
        queue.stage(vec![file.clone(), file]);
        assert_eq!(queue.drafts.len(), 1);
        assert_eq!(queue.drafts[0].bytes, 5);
        assert!(!queue.drafts[0].paused);
        queue.toggle(0);
        assert!(queue.drafts[0].paused);
        queue.toggle(0);
        assert!(!queue.drafts[0].paused);
        queue.remove(0);
        assert!(queue.drafts.is_empty());
    }

    #[test]
    fn directories_and_missing_files_are_not_staged() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing.bin");
        let mut queue = TransferState::default();
        queue.stage(vec![root.path().to_path_buf(), missing]);
        assert!(queue.drafts.is_empty());
        assert!(queue.error.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_never_staged() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("real.bin");
        let link = root.path().join("link.bin");
        std::fs::write(&file, b"hello").unwrap();
        symlink(&file, &link).unwrap();
        let mut queue = TransferState::default();
        queue.stage(vec![link]);
        assert!(queue.drafts.is_empty());
    }

    #[test]
    fn drafts_are_bounded() {
        let root = tempfile::tempdir().unwrap();
        let paths: Vec<PathBuf> = (0..MAX_DRAFTS + 2).map(|n| {
            let p = root.path().join(format!("{n}.bin"));
            std::fs::write(&p, b"x").unwrap();
            p
        }).collect();
        let mut queue = TransferState::default();
        queue.stage(paths);
        assert_eq!(queue.drafts.len(), MAX_DRAFTS);
        assert!(queue.error.is_some());
    }
}
