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

mod actions;
#[cfg(feature = "p2p-device-contacts-preview")]
mod contacts;
mod dialog;
#[cfg(feature = "p2p-inbox-preview")]
mod inbox;
#[cfg(feature = "p2p-device-ui")]
mod local_identity;
mod model;
#[cfg(feature = "p2p-native-receive-beta")]
mod native_receive;
#[cfg(feature = "p2p-native-send-beta")]
mod native_send;
#[cfg(feature = "p2p-device-ui")]
mod peer_pair;
#[cfg(feature = "messaging-w4-pilot")]
mod w4_pilot;

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
        #[cfg(all(windows, feature = "p2p-inbox-preview"))]
        if std::mem::take(&mut self.transfer.inbox.refresh_vault_copies) {
            self.tree.request_imported();
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
}

#[cfg(test)]
mod tests;
