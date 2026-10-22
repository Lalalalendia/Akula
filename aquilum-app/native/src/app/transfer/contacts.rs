//! Explicit opt-in UI observation of OS-pinned Iroh contacts. No network send.
use super::{App, Popup, TransferState};
use crate::ui::transfer::{self as transfer_ui, TransportContactRow, TransportContactView};
use aquilum_core::p2p_device_identity::peer_pin::{PeerState, TransportPeerContact, list_peers};
use masonry::app::RenderRoot;
use masonry::kurbo::Point;

fn map_rows(contacts: Vec<TransportPeerContact>) -> Vec<TransportContactRow> {
    contacts
        .into_iter()
        .map(|contact| TransportContactRow {
            full_endpoint_id: contact.endpoint_id_hex,
            revoked: contact.state == PeerState::Revoked,
        })
        .collect()
}

impl TransferState {
    /// Refresh at the moment the transfer dialog opens. Never treat a cached
    /// visual contact as a QUIC grant; Core must re-read before each operation.
    pub(super) fn refresh_transport_contacts(&mut self) {
        self.transport_contacts = match list_peers() {
            Ok(contacts) => TransportContactView {
                rows: map_rows(contacts),
                error: None,
            },
            Err(error) => TransportContactView {
                rows: Vec::new(),
                error: Some(error.to_string()),
            },
        };
    }
}

// Feature-gated variant: ordinary Akula retains the original dialog function.
impl App {
    pub(super) fn show_transfers(&mut self, root: &mut RenderRoot) {
        let (layer, dialog) = transfer_ui::dialog(
            &self.transfer.rows(),
            &self.transfer.device_rows(),
            &self.transfer.transport_contacts,
            self.transfer.choosing,
            self.transfer.error.as_deref(),
        );
        root.add_layer(layer, Point::ORIGIN);
        root.focus_on(Some(dialog.layer));
        self.popup = Some(Popup::Transfers(dialog));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_pin_rows_do_not_create_selectable_owner_devices() {
        let mut state = TransferState::default();
        let contact = TransportPeerContact {
            endpoint_id_hex: "a1".repeat(32),
            state: PeerState::Confirmed,
        };
        state.transport_contacts.rows = map_rows(vec![contact]);
        assert_eq!(state.transport_contacts.rows.len(), 1);
        assert!(!state.transport_contacts.rows[0].revoked);
        assert_eq!(state.transport_contacts.rows[0].full_endpoint_id.len(), 64);
        assert!(state.device_rows().is_empty());
        assert!(state.selected_peer.is_none());
    }
}
