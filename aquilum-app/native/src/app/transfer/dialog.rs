//! Transfer overlay construction, preserving W4 alongside opt-in native P2P.
use super::*;

impl App {
    pub(super) fn show_transfers(&mut self, root: &mut RenderRoot) {
        let identity_view = {
            #[cfg(feature = "p2p-device-ui")]
            { Some(&self.transfer.local_identity.view) }
            #[cfg(not(feature = "p2p-device-ui"))]
            { None }
        };
        let extras = transfer_ui::TransferExtras {
            identity: identity_view,
            #[cfg(feature = "p2p-device-ui")]
            peer: if matches!(
                self.transfer.local_identity.view.phase,
                transfer_ui::IdentityPhase::Ready(_)
            ) {
                Some(&self.transfer.peer_pair.view)
            } else {
                None
            },
            #[cfg(feature = "p2p-inbox-preview")]
            inbox: Some(&self.transfer.inbox.view),
            #[cfg(feature = "p2p-native-send-beta")]
            sender: Some(&self.transfer.sender.view),
            #[cfg(feature = "p2p-native-send-beta")]
            can_send: self.transfer.drafts.len() == 1 && !self.transfer.drafts[0].paused,
            #[cfg(feature = "p2p-native-receive-beta")]
            receiver: Some(&self.transfer.receiver.view),
            #[cfg(feature = "messaging-w4-pilot")]
            w4: Some(&self.transfer.w4_pilot.view),
        };
        #[cfg(feature = "p2p-device-contacts-preview")]
        let (layer, dialog) = transfer_ui::dialog(
            &self.transfer.rows(), &self.transfer.device_rows(),
            &self.transfer.transport_contacts, self.transfer.choosing,
            self.transfer.error.as_deref(), extras,
        );
        #[cfg(not(feature = "p2p-device-contacts-preview"))]
        let (layer, dialog) = transfer_ui::dialog(
            &self.transfer.rows(), &self.transfer.device_rows(),
            self.transfer.choosing, self.transfer.error.as_deref(), extras,
        );
        root.add_layer(layer, Point::ORIGIN);
        root.focus_on(Some(dialog.layer));
        self.popup = Some(Popup::Transfers(dialog));
    }

    pub(super) fn refresh_transfers(&mut self, root: &mut RenderRoot) {
        if let Some(Popup::Transfers(dialog)) = self.popup.take() {
            root.remove_layer(dialog.layer);
            self.show_transfers(root);
        }
    }
}
