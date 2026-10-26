//! Explicit opt-in UI observation of OS-pinned Iroh contacts. No network send.
use super::TransferState;
use crate::ui::transfer::{TransportContactRow, TransportContactView};
use aquilum_core::p2p_device_identity::peer_pin::{PeerState, TransportPeerContact, list_peers};

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
