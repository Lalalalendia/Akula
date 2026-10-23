//! Overlay construction is kept separate so the parent Transfer module does
//! not grow with experimental messaging panels.
use super::*;

impl App {
    pub(super) fn show_transfers(&mut self, root: &mut RenderRoot) {
        let identity_view = {
            #[cfg(feature = "p2p-device-ui")]
            { Some(&self.transfer.local_identity.view) }
            #[cfg(not(feature = "p2p-device-ui"))]
            { None }
        };
        #[cfg(all(feature = "p2p-device-contacts-preview", feature = "messaging-w4-pilot"))]
        let (layer, dialog) = transfer_ui::dialog(
            &self.transfer.rows(), &self.transfer.device_rows(),
            &self.transfer.transport_contacts, self.transfer.choosing,
            self.transfer.error.as_deref(), identity_view,
            Some(&self.transfer.w4_pilot.view),
        );
        #[cfg(all(feature = "p2p-device-contacts-preview", not(feature = "messaging-w4-pilot")))]
        let (layer, dialog) = transfer_ui::dialog(
            &self.transfer.rows(), &self.transfer.device_rows(),
            &self.transfer.transport_contacts, self.transfer.choosing,
            self.transfer.error.as_deref(), identity_view,
        );
        #[cfg(all(not(feature = "p2p-device-contacts-preview"), feature = "messaging-w4-pilot"))]
        let (layer, dialog) = transfer_ui::dialog(
            &self.transfer.rows(), &self.transfer.device_rows(),
            self.transfer.choosing, self.transfer.error.as_deref(), identity_view,
            Some(&self.transfer.w4_pilot.view),
        );
        #[cfg(all(not(feature = "p2p-device-contacts-preview"), not(feature = "messaging-w4-pilot")))]
        let (layer, dialog) = transfer_ui::dialog(
            &self.transfer.rows(), &self.transfer.device_rows(),
            self.transfer.choosing, self.transfer.error.as_deref(), identity_view,
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
