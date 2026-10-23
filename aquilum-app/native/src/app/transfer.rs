//! Local transfer drafts only. The network service, peer discovery, File Gate,
//! delivery receipts and verified progress remain unavailable in this UI.

use std::path::PathBuf;
use std::sync::Arc;

use aquilum_core::sync::DeviceId;
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};
use masonry::kurbo::Point;

use super::{App, Outcome, Popup};
use crate::ui::dialog::{self as ui_dialog, DialogAnswer};
use crate::ui::transfer::{self as transfer_ui, QueueAction};
use crate::ui::widgets::Pressed;

const MAX_DRAFTS: usize = 32;
const MAX_DEVICE_ROWS: usize = 32;

mod model;
mod dialog;
#[cfg(feature = "p2p-inbox-preview")]
mod inbox;
#[cfg(feature = "p2p-native-send-beta")]
mod native_send;
#[cfg(feature = "p2p-native-receive-beta")]
mod native_receive;
#[cfg(feature = "p2p-device-ui")]
mod peer_pair;
#[cfg(feature = "messaging-w4-pilot")]
mod w4_pilot;
#[cfg(feature = "p2p-device-contacts-preview")]
mod contacts;
#[cfg(feature = "p2p-device-ui")]
mod local_identity;

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
    #[cfg(feature = "p2p-device-contacts-preview")]
    transport_contacts: transfer_ui::TransportContactView,
    pub(super) selected_peer: Option<(DeviceId, [u8; 32])>,
    #[cfg(feature = "p2p-device-ui")]
    pub(super) peer_pair: peer_pair::PeerUi,
    #[cfg(feature = "p2p-inbox-preview")]
    pub(super) inbox: inbox::InboxPreview,
    #[cfg(feature = "p2p-native-send-beta")]
    pub(super) sender: native_send::NativeSender,
    #[cfg(feature = "p2p-native-receive-beta")]
    pub(super) receiver: native_receive::NativeReceiver,
    #[cfg(feature = "messaging-w4-pilot")]
    pub(super) w4_pilot: w4_pilot::PilotUi,
    #[cfg(feature = "p2p-device-ui")]
    pub(super) local_identity: local_identity::LocalIdentityUi,
}

impl App {
    pub(super) fn open_transfers(&mut self, root: &mut RenderRoot) {
        if self.popup.is_some() {
            return;
        }
        #[cfg(feature = "p2p-device-contacts-preview")]
        self.transfer.refresh_transport_contacts();
        #[cfg(feature = "p2p-inbox-preview")]
        self.begin_inbox_preview();
        self.show_transfers(root);
    }

    pub(super) fn on_transfer_picked(&mut self, root: &mut RenderRoot) {
        let mut changed = false;
        while let Ok(paths) = self.transfer_rx.try_recv() {
            self.transfer.choosing = false;
            self.transfer.stage(paths);
            changed = true;
        }
        #[cfg(feature = "p2p-device-ui")]
        if self.transfer.local_identity.poll() {
            changed = true;
        }
        #[cfg(feature = "p2p-device-ui")]
        if self.transfer.peer_pair.poll() {
            // A contact can be revoked while a dialog is open.
            #[cfg(feature = "p2p-device-contacts-preview")]
            self.transfer.refresh_transport_contacts();
            changed = true;
        }
        #[cfg(feature = "p2p-inbox-preview")]
        if self.transfer.inbox.poll() {
            changed = true;
        }
        #[cfg(feature = "p2p-native-send-beta")]
        if self.transfer.sender.poll() {
            changed = true;
        }
        #[cfg(feature = "p2p-native-receive-beta")]
        if self.transfer.receiver.poll() {
            if self.transfer.receiver.view.success {
                self.begin_inbox_preview();
            }
            changed = true;
        }
        #[cfg(feature = "messaging-w4-pilot")]
        if self.transfer.w4_pilot.poll() {
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
            #[cfg(feature = "p2p-native-send-beta")]
            self.transfer.sender.cancel();
            #[cfg(feature = "p2p-native-receive-beta")]
            self.transfer.receiver.cancel();
            ui_dialog::close(root, dialog.layer);
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
        #[cfg(feature = "p2p-inbox-preview")]
        if pressed && !refresh {
            if let Some((_, receipt)) = dialog.inbox_save_actions.iter()
                .find(|(button, _)| *button == id)
            {
                refresh = self.begin_inbox_save(receipt);
            }
        }
        #[cfg(feature = "p2p-native-send-beta")]
        if pressed && !refresh {
            if let Some((_, command)) = dialog.native_send_actions.iter()
                .find(|(button, _)| *button == id)
            {
                refresh = match command {
                    transfer_ui::NativeSendAction::Start(peer) => self.begin_native_send(peer),
                    transfer_ui::NativeSendAction::StartInternet(peer) => {
                        self.begin_native_send_internet(peer)
                    }
                    transfer_ui::NativeSendAction::Cancel => {
                        self.transfer.sender.cancel();
                        true
                    }
                };
            }
        }
        #[cfg(feature = "p2p-native-receive-beta")]
        if pressed && !refresh {
            if let Some((_, command)) = dialog.native_receive_actions.iter()
                .find(|(button, _)| *button == id)
            {
                refresh = match command {
                    transfer_ui::NativeReceiveAction::Start(peer) => self.begin_native_receive(peer),
                    transfer_ui::NativeReceiveAction::StartInternet(peer) => {
                        self.begin_native_receive_internet(peer)
                    }
                    transfer_ui::NativeReceiveAction::Cancel => {
                        self.transfer.receiver.cancel();
                        true
                    }
                };
            }
        }
        #[cfg(feature = "p2p-device-ui")]
        if pressed && !refresh {
            if let Some((_, command)) = dialog.identity_actions.iter()
                .find(|(button, _)| *button == id)
            {
                self.transfer.local_identity.action(
                    *command, Arc::clone(&self.workspace.wake),
                );
                refresh = true;
            }
        }
        #[cfg(feature = "p2p-device-ui")]
        if !refresh {
            if let Some(controls) = &dialog.peer_controls {
                refresh = self.transfer.peer_pair.handle(
                    controls, id, action, Arc::clone(&self.workspace.wake),
                );
            }
        }
        #[cfg(feature = "messaging-w4-pilot")]
        if pressed && !refresh {
            if let Some((_, command)) = dialog.w4_actions.iter()
                .find(|(button, _)| *button == id)
            {
                self.transfer.w4_pilot.action(*command, Arc::clone(&self.workspace.wake));
                refresh = true;
            }
        }
        self.popup = Some(Popup::Transfers(dialog));
        if refresh {
            self.refresh_transfers(root);
        }
        Outcome::default()
    }
}

#[cfg(test)]
mod tests;
