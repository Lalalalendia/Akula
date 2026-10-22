//! Transfer drafts only: no network, File Gate rights, or delivery receipts.

use std::path::PathBuf;
use std::sync::Arc;

use aquilum_core::sync::DeviceId;
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};

use super::{App, Outcome, Popup};
use crate::ui::dialog::{self, DialogAnswer};
use crate::ui::transfer::{self as transfer_ui, QueueAction};
use crate::ui::widgets::Pressed;

const MAX_DRAFTS: usize = 32;
const MAX_DEVICE_ROWS: usize = 32;
mod model;
#[cfg(feature = "p2p-device-contacts-preview")]
mod contacts;
/// Display-only owner registry placeholder; never a transfer grant.
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
    #[cfg(feature = "p2p-device-contacts-preview")]
    transport_contacts: transfer_ui::TransportContactView,
    pub(super) selected_peer: Option<(DeviceId, [u8; 32])>,
}

impl App {
    pub(super) fn open_transfers(&mut self, root: &mut RenderRoot) {
        if self.popup.is_some() {
            return;
        }
        #[cfg(feature = "p2p-device-contacts-preview")]
        self.transfer.refresh_transport_contacts();
        self.show_transfers(root);
    }

    #[cfg(not(feature = "p2p-device-contacts-preview"))]
    fn show_transfers(&mut self, root: &mut RenderRoot) {
        let (layer, dialog) = transfer_ui::dialog(
            &self.transfer.rows(), &self.transfer.device_rows(),
            self.transfer.choosing, self.transfer.error.as_deref(),
        );
        root.add_layer(layer, masonry::kurbo::Point::ORIGIN);
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
