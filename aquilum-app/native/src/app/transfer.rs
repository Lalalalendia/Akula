//! Local transfer drafts only. The network service, peer discovery, File Gate,
//! delivery receipts and verified progress remain unavailable in this UI.

use std::path::PathBuf;
use std::sync::Arc;

use aquilum_core::sync::DeviceId;
use masonry::app::RenderRoot;
use masonry::kurbo::Point;

use super::{App, Popup};
use crate::ui::transfer as transfer_ui;

const MAX_DRAFTS: usize = 32;
const MAX_DEVICE_ROWS: usize = 32;

mod model;
mod actions;
mod picker;
#[cfg(feature = "p2p-device-ui")]
mod peer_pair;
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
    pub(super) local_identity: local_identity::LocalIdentityUi,
    #[cfg(feature = "p2p-device-ui")]
    pub(super) peer_pair: peer_pair::PeerUi,
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
                if matches!(self.transfer.local_identity.view.phase, transfer_ui::IdentityPhase::Ready(_)) {
                    Some(&self.transfer.peer_pair.view)
                } else {
                    None
                }
            }
            #[cfg(not(feature = "p2p-device-ui"))]
            { None }
        };
        #[cfg(feature = "p2p-device-contacts-preview")]
        let (layer, dialog) = transfer_ui::dialog(
            &self.transfer.rows(), &self.transfer.device_rows(),
            &self.transfer.transport_contacts, self.transfer.choosing,
            self.transfer.error.as_deref(), identity_view, peer_view,
        );
        #[cfg(not(feature = "p2p-device-contacts-preview"))]
        let (layer, dialog) = transfer_ui::dialog(
            &self.transfer.rows(), &self.transfer.device_rows(),
            self.transfer.choosing, self.transfer.error.as_deref(), identity_view, peer_view,
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
            changed = true;
        }
        if changed {
            self.refresh_transfers(root);
        }
    }
}

#[cfg(test)]
mod tests;
