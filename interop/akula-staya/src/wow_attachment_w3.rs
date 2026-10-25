//! Opt-in WOW W3 cryptographic envelope for one already-approved Akula
//! attachment notice. The byte payload is not a read grant and this adapter
//! neither sends bytes nor claims a W4/Inbox/File Gate receipt.
use akula_wire_lab::{crypto_w3, trust_w3::TrustRegistry};
use aquilum_core::p2p_attachment_contract::{AttachmentNoticeV1, AttachmentError, NOTICE_BYTES};
use aquilum_core::p2p_offer_lab::{OfferV1, DecisionV1, PeerBinding};
use ed25519_dalek::{SigningKey, VerifyingKey};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum WowAttachmentError {
    Crypto(crypto_w3::CryptoError),
    UntrustedOrInvalidNotice,
    WrongMessageId,
}
impl From<crypto_w3::CryptoError> for WowAttachmentError {
    fn from(e: crypto_w3::CryptoError) -> Self { Self::Crypto(e) }
}
impl From<AttachmentError> for WowAttachmentError {
    fn from(_: AttachmentError) -> Self { Self::UntrustedOrInvalidNotice }
}

/// Real COSE Ed25519 inside HPKE (W3), bound to the exact stable notice ID,
/// recipient and expiry. Keys must be obtained from authenticated enrollment.
pub fn seal_selected_notice(
    notice: &AttachmentNoticeV1,
    signing_key: &SigningKey,
    recipient_public: &crypto_w3::RecipientPublic,
) -> Result<Vec<u8>, WowAttachmentError> {
    Ok(crypto_w3::seal(
        signing_key, recipient_public, notice.message_id(),
        notice.wow_recipient_id(), notice.expires_unix(), &notice.encode()
    )?)
}

/// The supplied trust registry is only an in-memory lab registry, not
/// production enrollment. The expected peers MUST be independently pinned.
/// Expiry is checked twice (W3 header and original Akula Offer/Decision).
/// Durable replay suppression is a future required separate boundary.
pub fn open_selected_notice(
    packet: &[u8],
    recipient_secret: &crypto_w3::RecipientSecret,
    enrolled_sender_key: &VerifyingKey,
    trust: &TrustRegistry,
    enrolled_sender_epoch: u64,
    offer: &OfferV1,
    decision: &DecisionV1,
    authenticated_sender: PeerBinding,
    enrolled_receiver: PeerBinding,
    now: u64,
) -> Result<AttachmentNoticeV1, WowAttachmentError> {
    let (message_id, plain) = crypto_w3::open_with_trust(
        packet,
        crypto_w3::RecipientContext {
            secret: recipient_secret,
            expected_sender: enrolled_sender_key,
            expected_recipient: *enrolled_receiver.device_id.as_uuid().as_bytes(),
            now,
        },
        trust,
        *authenticated_sender.device_id.as_uuid().as_bytes(),
        enrolled_sender_epoch,
    )?;
    if plain.len() != NOTICE_BYTES {
        return Err(WowAttachmentError::UntrustedOrInvalidNotice);
    }
    let notice = AttachmentNoticeV1::decode_bound(
        &plain, offer, decision, authenticated_sender, enrolled_receiver, now
    )?;
    if message_id != notice.message_id() {
        return Err(WowAttachmentError::WrongMessageId);
    }
    Ok(notice)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aquilum_core::p2p_offer_lab::{DecisionAction, OfferedFile, SCHEMA};
    use aquilum_core::sync::DeviceId;
    use uuid::Uuid;

    const NOW: u64 = 1_800_000_000;
    fn peer(id: u128, endpoint: u8) -> PeerBinding {
        PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(id)),
            endpoint_id: [endpoint; 32]
        }
    }
    #[test]
    fn real_w3_hpke_ed25519_roundtrip_wrong_recipient_revocation_tamper_and_expiry() {
        let sender = peer(17, 11);
        let receiver = peer(23, 27);
        let offer = OfferV1 {
            schema: SCHEMA, offer_id: Uuid::from_u128(42), nonce: [91; 16],
            sender, receiver, created_unix: NOW, expires_unix: NOW+120,
            files: vec![OfferedFile {
                label: "photo.bin".into(), bytes: 5*1024*1024,
                blake3_root: [3;32],
            }]
        };
        let decision = offer.receiver_decision(
            DecisionAction::Accept, vec![0], NOW, sender, receiver
        ).unwrap();
        let notice = AttachmentNoticeV1::from_accepted(
            &offer, &decision, sender, receiver, 0, NOW+1
        ).unwrap();
        let signing_key = SigningKey::from_bytes(&[37;32]);
        let verifying_key = signing_key.verifying_key();
        let (secret, public) = crypto_w3::new_recipient();
        let mut trust = TrustRegistry::new();
        trust.enroll(*sender.device_id.as_uuid().as_bytes(), 7, verifying_key).unwrap();
        let packet = seal_selected_notice(&notice, &signing_key, &public).unwrap();
        let open = |packet: &[u8], trust: &TrustRegistry, time: u64, receiver: PeerBinding| {
            open_selected_notice(
                packet, &secret, &verifying_key, trust, 7, &offer, &decision,
                sender, receiver, time
            )
        };
        assert_eq!(open(&packet, &trust, NOW+1, receiver), Ok(notice));
        let mut damaged = packet.clone();
        let last = damaged.len()-1;
        damaged[last] ^= 1;
        assert!(open(&damaged, &trust, NOW+1, receiver).is_err());
        assert!(open(&packet, &trust, NOW+1, peer(99,27)).is_err());
        assert!(open(&packet, &trust, NOW+120, receiver).is_err());
        trust.revoke(*sender.device_id.as_uuid().as_bytes()).unwrap();
        assert!(open(&packet, &trust, NOW+1, receiver).is_err());
        let (other_secret, _) = crypto_w3::new_recipient();
        assert!(open_selected_notice(
            &packet, &other_secret, &verifying_key, &TrustRegistry::new(),
            7, &offer, &decision, sender, receiver, NOW+1
        ).is_err());
    }
}
