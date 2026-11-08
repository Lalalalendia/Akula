//! Preflight match of accepted Offer metadata to independently observed Inbox facts.
//! Not an authenticated receipt, persistence format, UI name grant or filesystem authority.
//! The caller MUST derive peer bindings from live authenticated QUIC/enrollment and
//! facts from a rehashed, pinned local Inbox handle; arbitrary JSON/SQLite is not proof.
use super::{OfferError, OfferSession, PeerBinding, Phase};
use uuid::Uuid;

pub mod signed;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservedLocalReceipt {
    pub receipt_id: Uuid,
    pub sender_endpoint: [u8; 32],
    pub receiver_endpoint: [u8; 32],
    pub bytes: u64,
    pub blake3_root: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingError {
    NotAccepted,
    UnselectedFile,
    ReceiptMismatch,
    Offer(OfferError),
}

/// Transient comparison result only. The label stays private to prevent
/// accidental promotion to an authenticated, openable Native attachment.
/// Durable receipt binding and replay protection are NOT implemented here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataMatchCandidate {
    offer_id: Uuid,
    receipt_id: Uuid,
    file_index: u8,
    manifest_commitment: [u8; 32],
    _untrusted_display_label: String,
}

/// An accepted sender-side session remembers the actual selected file indices.
/// Merely supplying a new, valid-looking DecisionV1 cannot expand that selection.
/// This function does not open files, persist grants, display labels or authorize
/// import; a later durable proof must bind offer/consent/receipt atomically.
pub fn match_accepted_local_receipt(
    session: &OfferSession,
    selected_index: u8,
    authenticated_sender: PeerBinding,
    trusted_local_receiver: PeerBinding,
    observed: ObservedLocalReceipt,
    now: u64,
) -> Result<MetadataMatchCandidate, BindingError> {
    if session.phase != Phase::Accepted {
        return Err(BindingError::NotAccepted);
    }
    if !session
        .accepted_indices
        .as_deref()
        .is_some_and(|indices| indices.contains(&selected_index))
    {
        return Err(BindingError::UnselectedFile);
    }
    session.offer.validate(now).map_err(BindingError::Offer)?;
    session
        .offer
        .check_peers(authenticated_sender, trusted_local_receiver)
        .map_err(BindingError::Offer)?;
    let file = session
        .offer
        .files
        .get(usize::from(selected_index))
        .ok_or(BindingError::UnselectedFile)?;
    if observed.receipt_id.is_nil()
        || observed.sender_endpoint != authenticated_sender.endpoint_id
        || observed.receiver_endpoint != trusted_local_receiver.endpoint_id
        || observed.bytes != file.bytes
        || observed.blake3_root != file.blake3_root
    {
        return Err(BindingError::ReceiptMismatch);
    }
    Ok(MetadataMatchCandidate {
        offer_id: session.offer.offer_id,
        receipt_id: observed.receipt_id,
        file_index: selected_index,
        manifest_commitment: session.commitment,
        _untrusted_display_label: file.label.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p2p_offer_lab::{
        DecisionAction, OfferSession, OfferV1, OfferedFile, Outcome, SCHEMA,
    };
    use crate::sync::DeviceId;

    const NOW: u64 = 1_800_000_000;

    fn peer(id: u128, key: u8) -> PeerBinding {
        PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(id)),
            endpoint_id: [key; 32],
        }
    }

    fn offer() -> OfferV1 {
        OfferV1 {
            schema: SCHEMA,
            offer_id: Uuid::from_u128(123),
            nonce: [17; 16],
            sender: peer(1, 3),
            receiver: peer(2, 4),
            created_unix: NOW,
            expires_unix: NOW + 120,
            files: vec![
                OfferedFile {
                    label: "first.txt".into(),
                    bytes: 5,
                    blake3_root: *blake3::hash(b"alpha").as_bytes(),
                },
                OfferedFile {
                    label: "second.bin".into(),
                    bytes: 4,
                    blake3_root: *blake3::hash(b"beta").as_bytes(),
                },
            ],
        }
    }

    fn accepted(offer: &OfferV1, indices: Vec<u8>) -> OfferSession {
        let decision = offer
            .receiver_decision(
                DecisionAction::Accept,
                indices,
                NOW + 1,
                offer.sender,
                offer.receiver,
            )
            .unwrap();
        let mut session =
            OfferSession::new_sender(offer.clone(), NOW, offer.sender, offer.receiver).unwrap();
        assert!(matches!(
            session.resolve(&decision, offer.receiver, NOW + 1).unwrap(),
            Outcome::Selected(_)
        ));
        assert_eq!(
            session.resolve(&decision, offer.receiver, NOW + 2),
            Err(OfferError::AlreadyResolved)
        );
        session
    }

    fn observed(offer: &OfferV1, index: usize) -> ObservedLocalReceipt {
        let file = &offer.files[index];
        ObservedLocalReceipt {
            receipt_id: Uuid::from_u128(987),
            sender_endpoint: offer.sender.endpoint_id,
            receiver_endpoint: offer.receiver.endpoint_id,
            bytes: file.bytes,
            blake3_root: file.blake3_root,
        }
    }

    fn checked(
        session: &OfferSession,
        offer: &OfferV1,
        index: u8,
        receipt: ObservedLocalReceipt,
        now: u64,
    ) -> Result<MetadataMatchCandidate, BindingError> {
        match_accepted_local_receipt(session, index, offer.sender, offer.receiver, receipt, now)
    }

    #[test]
    fn accepted_index_matches_only_its_observed_receipt() {
        let offer = offer();
        let session = accepted(&offer, vec![1]);
        let matched = checked(&session, &offer, 1, observed(&offer, 1), NOW + 2).unwrap();
        assert_eq!(matched.offer_id, offer.offer_id);
        assert_eq!(matched.receipt_id, Uuid::from_u128(987));
        assert_eq!(matched.file_index, 1);
        assert_eq!(matched.manifest_commitment, offer.commitment());
        assert_eq!(matched._untrusted_display_label, "second.bin");
        assert_eq!(
            checked(&session, &offer, 0, observed(&offer, 0), NOW + 2),
            Err(BindingError::UnselectedFile)
        );
        assert_eq!(
            checked(&session, &offer, 2, observed(&offer, 1), NOW + 2),
            Err(BindingError::UnselectedFile)
        );
    }

    #[test]
    fn cold_restart_without_accepted_session_is_fail_closed() {
        let offer = offer();
        let pending =
            OfferSession::new_sender(offer.clone(), NOW, offer.sender, offer.receiver).unwrap();
        assert_eq!(
            checked(&pending, &offer, 0, observed(&offer, 0), NOW + 2),
            Err(BindingError::NotAccepted)
        );
        let mut declined =
            OfferSession::new_sender(offer.clone(), NOW, offer.sender, offer.receiver).unwrap();
        let decision = offer
            .receiver_decision(
                DecisionAction::Decline,
                vec![],
                NOW + 1,
                offer.sender,
                offer.receiver,
            )
            .unwrap();
        assert_eq!(
            declined.resolve(&decision, offer.receiver, NOW + 1),
            Ok(Outcome::Declined)
        );
        assert_eq!(
            checked(&declined, &offer, 0, observed(&offer, 0), NOW + 2),
            Err(BindingError::NotAccepted)
        );
    }

    #[test]
    fn wrong_live_sender_receiver_and_local_endpoint_fail_closed() {
        let offer = offer();
        let session = accepted(&offer, vec![0]);
        let receipt = observed(&offer, 0);
        assert_eq!(
            match_accepted_local_receipt(&session, 0, peer(3, 3), offer.receiver, receipt, NOW + 2),
            Err(BindingError::Offer(OfferError::WrongIdentity))
        );
        assert_eq!(
            match_accepted_local_receipt(&session, 0, offer.sender, peer(4, 4), receipt, NOW + 2),
            Err(BindingError::Offer(OfferError::WrongIdentity))
        );
        let mut wrong = receipt;
        wrong.sender_endpoint = [13; 32];
        assert_eq!(
            checked(&session, &offer, 0, wrong, NOW + 2),
            Err(BindingError::ReceiptMismatch)
        );
        wrong = receipt;
        wrong.receiver_endpoint = [14; 32];
        assert_eq!(
            checked(&session, &offer, 0, wrong, NOW + 2),
            Err(BindingError::ReceiptMismatch)
        );
    }

    #[test]
    fn changed_size_digest_and_missing_receipt_id_fail_closed() {
        let offer = offer();
        let session = accepted(&offer, vec![0]);
        let receipt = observed(&offer, 0);
        let mut changed = receipt;
        changed.bytes += 1;
        assert_eq!(
            checked(&session, &offer, 0, changed, NOW + 2),
            Err(BindingError::ReceiptMismatch)
        );
        changed = receipt;
        changed.blake3_root = [7; 32];
        assert_eq!(
            checked(&session, &offer, 0, changed, NOW + 2),
            Err(BindingError::ReceiptMismatch)
        );
        changed = receipt;
        changed.receipt_id = Uuid::nil();
        assert_eq!(
            checked(&session, &offer, 0, changed, NOW + 2),
            Err(BindingError::ReceiptMismatch)
        );
    }

    #[test]
    fn cross_selected_files_expired_offers_and_other_peers_fail_closed() {
        let offer = offer();
        let session = accepted(&offer, vec![0, 1]);
        assert_eq!(
            checked(&session, &offer, 1, observed(&offer, 0), NOW + 2),
            Err(BindingError::ReceiptMismatch)
        );
        assert_eq!(
            checked(&session, &offer, 0, observed(&offer, 0), NOW + 121),
            Err(BindingError::Offer(OfferError::Expired))
        );
        let mut other = offer.clone();
        other.offer_id = Uuid::from_u128(124);
        other.sender = peer(9, 9);
        let other_session = accepted(&other, vec![0]);
        assert_eq!(
            checked(&other_session, &other, 0, observed(&offer, 0), NOW + 2),
            Err(BindingError::ReceiptMismatch)
        );
    }

    #[test]
    fn malicious_labels_cannot_enter_the_candidate_contract() {
        for bad in [
            "../readme.txt",
            "CON.txt",
            "folder\\item.txt",
            "C:payload.exe",
            "trailing.",
            "mis\u{202e}exe.txt",
        ] {
            let mut candidate = offer();
            candidate.files[0].label = bad.into();
            assert!(matches!(
                OfferSession::new_sender(
                    candidate.clone(),
                    NOW,
                    candidate.sender,
                    candidate.receiver
                ),
                Err(OfferError::InvalidMetadata)
            ));
        }
    }
}
