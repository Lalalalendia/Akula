//! Transfer overlay construction, preserving W4 alongside opt-in native P2P.
use super::*;
mod context;

impl App {
    pub(super) fn show_transfers(&mut self, root: &mut RenderRoot) {
        let extras = self.transfer.dialog_extras();
        #[cfg(feature = "p2p-device-contacts-preview")]
        let (layer, dialog) = transfer_ui::dialog(
            &self.transfer.rows(),
            &self.transfer.device_rows(),
            &self.transfer.transport_contacts,
            self.transfer.choosing,
            self.transfer.error.as_deref(),
            extras,
        );
        #[cfg(not(feature = "p2p-device-contacts-preview"))]
        let (layer, dialog) = transfer_ui::dialog(
            &self.transfer.rows(),
            &self.transfer.device_rows(),
            self.transfer.choosing,
            self.transfer.error.as_deref(),
            extras,
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
