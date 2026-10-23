//! Local transfer drafts only. The network service, peer discovery, File Gate,
//! delivery receipts and verified progress remain unavailable in this UI.

use std::path::PathBuf;
use std::sync::Arc;

use aquilum_core::sync::DeviceId;
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};
use masonry::kurbo::Point;

use super::{App, Outcome, Popup};
use crate::ui::dialog::{self, DialogAnswer};
use crate::ui::transfer::{self as transfer_ui, QueueAction};
use crate::ui::widgets::Pressed;

const MAX_DRAFTS: usize = 32;
const MAX_DEVICE_ROWS: usize = 32;

mod model;
#[cfg(feature = "p2p-inbox-preview")]
mod inbox;
#[cfg(feature = "p2p-native-send-beta")]
mod native_send;
#[cfg(feature = "p2p-native-receive-beta")]
mod native_receive;
#[cfg(feature = "p2p-device-contacts-preview")]
mod contacts;
#[cfg(feature = "p2p-device-ui")]
mod local_identity;
#[cfg(feature = "p2p-device-ui")]
mod peer_pair;

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
    pub(super) local_identity: local_identity::LocalIdentityUi,
    #[cfg(feature = "p2p-device-ui")]
    pub(super) peer_pair: peer_pair::PeerUi,
    #[cfg(feature = "p2p-inbox-preview")]
    pub(super) inbox: inbox::InboxPreview,
    #[cfg(feature = "p2p-native-send-beta")]
    pub(super) sender: native_send::NativeSender,
    #[cfg(feature = "p2p-native-receive-beta")]
    pub(super) receiver: native_receive::NativeReceiver,
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

    fn show_transfers(&mut self, root: &mut RenderRoot) {
        let identity_view = {
            #[cfg(feature = "p2p-device-ui")]
            { Some(&self.transfer.local_identity.view) }
            #[cfg(not(feature = "p2p-device-ui"))]
            { None }
        };
        let peer_view = {
            #[cfg(feature = "p2p-device-ui")]
            {
                if matches!(
                    self.transfer.local_identity.view.phase,
                    transfer_ui::IdentityPhase::Ready(_)
                ) {
                    Some(&self.transfer.peer_pair.view)
                } else {
                    None
                }
            }
            #[cfg(not(feature = "p2p-device-ui"))]
            {
                None
            }
        };
        #[cfg(feature = "p2p-inbox-preview")]
        let inbox_view = Some(&self.transfer.inbox.view);
        #[cfg(not(feature = "p2p-inbox-preview"))]
        let inbox_view = None;
        // Rust cannot conditionally remove individual call arguments.
        // Keep feature combinations explicit so default-off still compiles.
        #[cfg(feature = "p2p-native-receive-beta")]
        let (layer, dialog) = transfer_ui::dialog(
            &self.transfer.rows(), &self.transfer.device_rows(),
            &self.transfer.transport_contacts, self.transfer.choosing,
            self.transfer.error.as_deref(), identity_view, peer_view, inbox_view,
            Some(&self.transfer.sender.view),
            self.transfer.drafts.len() == 1 && !self.transfer.drafts[0].paused,
            Some(&self.transfer.receiver.view),
        );
        #[cfg(all(feature = "p2p-device-contacts-preview", feature = "p2p-native-send-beta", not(feature = "p2p-native-receive-beta")))]
        let (layer, dialog) = transfer_ui::dialog(
            &self.transfer.rows(), &self.transfer.device_rows(),
            &self.transfer.transport_contacts, self.transfer.choosing,
            self.transfer.error.as_deref(), identity_view, peer_view, inbox_view,
            Some(&self.transfer.sender.view),
            self.transfer.drafts.len() == 1 && !self.transfer.drafts[0].paused,
        );
        #[cfg(all(feature = "p2p-device-contacts-preview", not(feature = "p2p-native-send-beta")))]
        let (layer, dialog) = transfer_ui::dialog(
            &self.transfer.rows(), &self.transfer.device_rows(),
            &self.transfer.transport_contacts, self.transfer.choosing,
            self.transfer.error.as_deref(), identity_view, peer_view, inbox_view,
        );
        #[cfg(not(feature = "p2p-device-contacts-preview"))]
        let (layer, dialog) = transfer_ui::dialog(
            &self.transfer.rows(), &self.transfer.device_rows(),
            self.transfer.choosing, self.transfer.error.as_deref(), identity_view, peer_view, inbox_view,
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
        #[cfg(feature = "p2p-device-ui")]
        if self.transfer.local_identity.poll() {
            changed = true;
        }
        #[cfg(feature = "p2p-device-ui")]
        if self.transfer.peer_pair.poll() {
            // Refresh the same protected OS contacts after a confirmed pin
            // or revocation; transfer buttons must reflect the new authority.
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
            if let Some((_, peer)) = dialog.native_send_actions.iter()
                .find(|(button, _)| *button == id)
            {
                refresh = match peer {
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
                    controls,
                    id,
                    action,
                    Arc::clone(&self.workspace.wake),
                );
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
