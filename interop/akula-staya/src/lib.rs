//! Real Akula OfferSession -> actual Staya QueueSelection adapter.
//!
//! This is a reusable, opt-in interoperability seam, NOT a transfer grant.
//! It compiles with the real Staya crate at an immutable source revision.
//! Callers MUST independently authenticate Iroh remote_id and Akula enrollment.
//! The returned queue contains roots and sizes only: no paths or write rights.

use aquilum_core::p2p_offer_lab::{
    DecisionV1, OfferError, OfferSession, OfferV1, Outcome, PeerBinding,
};
#[cfg(feature = "p2p-live-identity")]
pub mod live_identity;

use staya_transfer::{
    ContentId, Manifest,
    queue::{QueueError, QueueSelection, SelectedFile},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BridgeError {
    Offer(OfferError),
    Declined,
    ForgedOrReorderedSelection,
    InvalidInvitation,
    Staya(QueueError),
}

/// A Staya queue constructed from the exact Akula receiver-approved subset.
/// Neither queue nor its commitment is a network GET authorization.
pub struct PreparedBatch {
    pub queue: QueueSelection,
    pub offer_indexes: Vec<usize>,
    pub offer_commitment: [u8; 32],
}

/// Materialize only an already-validated Akula decision using the ACTUAL
/// staya-transfer crate, preserving root, size, and approved object order.
/// Never use untrusted OfferV1 identities as authenticated_* arguments.
pub fn prepare_staya_batch(
    offer: OfferV1,
    decision: &DecisionV1,
    authenticated_sender: PeerBinding,
    authenticated_receiver: PeerBinding,
    now: u64,
) -> Result<PreparedBatch, BridgeError> {
    let mut session = OfferSession::new_sender(
        offer.clone(),
        now,
        authenticated_sender,
        authenticated_receiver,
    )
    .map_err(BridgeError::Offer)?;
    let Outcome::Selected(selected) = session
        .resolve(decision, authenticated_receiver, now)
        .map_err(BridgeError::Offer)?
    else {
        return Err(BridgeError::Declined);
    };

    let mut entries = Vec::with_capacity(selected.files.len());
    let mut offer_indexes = Vec::with_capacity(selected.files.len());
    let mut cursor = 0usize;
    for (index, file) in offer.files.iter().enumerate() {
        if selected.files.get(cursor) == Some(file) {
            // The real Staya queue only permits restricted ASCII labels.
            // Display names from Akula never become filesystem paths here.
            entries.push(SelectedFile {
                label: format!("object_{index:02}.bin"),
                manifest: Manifest {
                    id: ContentId(file.blake3_root),
                    size: file.bytes,
                },
            });
            offer_indexes.push(index);
            cursor += 1;
        }
    }
    if entries.is_empty() || cursor != selected.files.len() {
        return Err(BridgeError::ForgedOrReorderedSelection);
    }
    let queue = QueueSelection::new(entries).map_err(BridgeError::Staya)?;
    Ok(PreparedBatch {
        queue,
        offer_indexes,
        offer_commitment: selected.commitment,
    })
}

/// Verify the ACTUAL serialized Staya invitation against the approved queue
/// before the receiver is allowed to consume it. The expected sender key
/// MUST be independently obtained from trusted enrollment or out-of-band
/// operator verification; reading it from the same invitation is NOT trust.
pub fn verify_staya_invitation(
    prepared: &PreparedBatch,
    invitation: &str,
    expected_sender_endpoint: [u8; 32],
) -> Result<(), BridgeError> {
    if invitation.is_empty() || invitation.len() > 8192 {
        return Err(BridgeError::InvalidInvitation);
    }
    let fields: Vec<&str> = invitation.split_whitespace().collect();
    let selected = prepared.queue.entries();
    if fields.first() != Some(&"CONNECT_BATCH")
        || fields.len() != 4 + selected.len() * 3
        || fields[3].parse::<usize>().ok() != Some(selected.len())
        || fields[2] != hex(&prepared.queue.commitment())
    {
        return Err(BridgeError::InvalidInvitation);
    }
    let ticket = decode_hex(fields[1]).ok_or(BridgeError::InvalidInvitation)?;
    let addr: iroh::EndpointAddr =
        postcard::from_bytes(&ticket).map_err(|_| BridgeError::InvalidInvitation)?;
    if addr.is_empty() || addr.id.as_bytes() != &expected_sender_endpoint {
        return Err(BridgeError::InvalidInvitation);
    }
    for (slot, entry) in selected.iter().enumerate() {
        let offset = 4 + slot * 3;
        if fields[offset] != entry.label
            || fields[offset + 1] != hex(&entry.manifest.id.0)
            || fields[offset + 2].parse::<u64>().ok() != Some(entry.manifest.size)
        {
            return Err(BridgeError::InvalidInvitation);
        }
    }
    Ok(())
}

fn hex(data: &[u8]) -> String {
    data.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.is_empty() || value.len() > 8192 || !value.len().is_multiple_of(2) {
        return None;
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let token = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(token, 16).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aquilum_core::p2p_offer_lab::{DecisionAction, OfferedFile, PeerBinding, SCHEMA};
    use aquilum_core::sync::DeviceId;
    use uuid::Uuid;

    const NOW: u64 = 1_800_000_000;

    fn peers() -> (PeerBinding, PeerBinding) {
        (
            PeerBinding {
                device_id: DeviceId::from_uuid(Uuid::from_u128(1)),
                endpoint_id: [23; 32],
            },
            PeerBinding {
                device_id: DeviceId::from_uuid(Uuid::from_u128(2)),
                endpoint_id: [47; 32],
            },
        )
    }

    fn offer() -> OfferV1 {
        let (sender, receiver) = peers();
        OfferV1 {
            schema: SCHEMA,
            offer_id: Uuid::from_u128(42),
            nonce: [7; 16],
            sender,
            receiver,
            created_unix: NOW,
            expires_unix: NOW + 120,
            files: vec![
                OfferedFile {
                    label: "Длинное русское имя документа.txt".into(),
                    bytes: 1,
                    blake3_root: *blake3::hash(b"a").as_bytes(),
                },
                OfferedFile {
                    label: "not-selected.txt".into(),
                    bytes: 2,
                    blake3_root: *blake3::hash(b"bb").as_bytes(),
                },
                OfferedFile {
                    label: "Архив.dat".into(),
                    bytes: 3,
                    blake3_root: *blake3::hash(b"ccc").as_bytes(),
                },
            ],
        }
    }

    #[test]
    fn real_staya_queue_excludes_unselected_roots() {
        let offer = offer();
        let (sender, receiver) = peers();
        let d = offer
            .receiver_decision(DecisionAction::Accept, vec![0, 2], NOW, sender, receiver)
            .unwrap();
        let prepared = prepare_staya_batch(offer.clone(), &d, sender, receiver, NOW + 1).unwrap();
        assert_eq!(prepared.offer_indexes, [0, 2]);
        assert_eq!(prepared.queue.entries().len(), 2);
        assert_eq!(prepared.queue.entries()[0].label, "object_00.bin");
        assert_eq!(prepared.queue.entries()[1].label, "object_02.bin");
        assert_eq!(
            prepared.queue.entries()[0].manifest.id.0,
            offer.files[0].blake3_root
        );
        assert_eq!(
            prepared.queue.entries()[1].manifest.id.0,
            offer.files[2].blake3_root
        );
        assert!(
            prepared
                .queue
                .entries()
                .iter()
                .all(|file| file.manifest.id.0 != offer.files[1].blake3_root)
        );
    }

    #[test]
    fn forged_or_wrong_peer_decision_cannot_create_staya_queue() {
        let offer = offer();
        let (sender, receiver) = peers();
        let mut d = offer
            .receiver_decision(DecisionAction::Accept, vec![0], NOW, sender, receiver)
            .unwrap();
        d.manifest_commitment = [0x55; 32];
        assert!(matches!(
            prepare_staya_batch(offer.clone(), &d, sender, receiver, NOW + 1),
            Err(BridgeError::Offer(OfferError::WrongCommitment))
        ));
        let d = offer
            .receiver_decision(DecisionAction::Accept, vec![0], NOW, sender, receiver)
            .unwrap();
        let fake_receiver = PeerBinding {
            endpoint_id: [99; 32],
            ..receiver
        };
        assert!(matches!(
            prepare_staya_batch(offer.clone(), &d, sender, fake_receiver, NOW + 1),
            Err(BridgeError::Offer(OfferError::WrongIdentity))
        ));
        let decline = offer
            .receiver_decision(DecisionAction::Decline, vec![], NOW, sender, receiver)
            .unwrap();
        assert!(matches!(
            prepare_staya_batch(offer, &decline, sender, receiver, NOW + 1),
            Err(BridgeError::Declined)
        ));
    }
}
