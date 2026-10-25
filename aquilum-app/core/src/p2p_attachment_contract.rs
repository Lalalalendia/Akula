//! Akula/Core attachment *metadata* contract for WOW notices and Staya ranges.
//!
//! A receiver-approved OfferSession identifies the precise immutable object,
//! peers and expiry. Approval is NOT a signed Akula read grant: no method in
//! this module may open a source, listen on QUIC, write a Vault object, assert
//! File Gate APPLIED, or sign a W4 receipt. The caller must separately verify
//! the authentic enrollment, object rights and signed authority before GET.
//!
//! The fixed 201-byte payload is intended to live *inside* a separately
//! encrypted/authenticated WOW W3 message, never as untrusted file authority.
#![forbid(unsafe_code)]
use crate::p2p_offer_lab::{
    DecisionV1, OfferError, OfferSession, OfferV1, Outcome, PeerBinding,
};
use uuid::Uuid;

pub const VERSION_MAGIC: [u8; 8] = *b"AKATTV01";
pub const NOTICE_BYTES: usize = 201;
pub const STAGE_BLOCK_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttachmentError {
    Offer(OfferError),
    NotSelected,
    WrongNotice,
    InvalidIndex,
}

impl From<OfferError> for AttachmentError {
    fn from(value: OfferError) -> Self { Self::Offer(value) }
}

/// The exact content selected by a receiver, not an authorization token.
/// No URLs, bearer grants, filenames or local paths may appear on this wire.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttachmentNoticeV1 {
    offer_id: Uuid,
    commitment: [u8; 32],
    sender: PeerBinding,
    receiver: PeerBinding,
    index: u8,
    size: u64,
    root: [u8; 32],
    expires_unix: u64,
}

impl AttachmentNoticeV1 {
    /// Derive a single selected object using the existing real Akula
    /// OfferSession. The peer values must come from verified local enrollment,
    /// NOT from untrusted WOW bytes or a remote advertisement.
    pub fn from_accepted(
        offer: &OfferV1,
        decision: &DecisionV1,
        authenticated_sender: PeerBinding,
        enrolled_receiver: PeerBinding,
        index: u8,
        now: u64,
    ) -> Result<Self, AttachmentError> {
        let mut session = OfferSession::new_sender(
            offer.clone(), now, authenticated_sender, enrolled_receiver
        )?;
        let Outcome::Selected(_selection) = session.resolve(
            decision, enrolled_receiver, now
        )? else {
            return Err(AttachmentError::NotSelected);
        };
        let Some(file) = offer.files.get(index as usize) else {
            return Err(AttachmentError::InvalidIndex);
        };
        if !decision.selected.contains(&index) {
            return Err(AttachmentError::NotSelected);
        }
        Ok(Self {
            offer_id: offer.offer_id,
            commitment: offer.commitment(),
            sender: authenticated_sender,
            receiver: enrolled_receiver,
            index,
            size: file.bytes,
            root: file.blake3_root,
            expires_unix: offer.expires_unix,
        })
    }

    pub fn offer_id(&self) -> Uuid { self.offer_id }
    pub fn commitment(&self) -> [u8; 32] { self.commitment }
    pub fn sender(&self) -> PeerBinding { self.sender }
    pub fn receiver(&self) -> PeerBinding { self.receiver }
    pub fn selected_index(&self) -> u8 { self.index }
    pub fn size(&self) -> u64 { self.size }
    pub fn root(&self) -> [u8; 32] { self.root }
    pub fn expires_unix(&self) -> u64 { self.expires_unix }

    /// Replay-stable, domain-separated ID for this *one* offered object. The
    /// WOW transport needs its independent replay and epoch policy as well.
    pub fn message_id(&self) -> [u8; 16] {
        let mut h = blake3::Hasher::new_derive_key("Akula WOW attachment message v1");
        h.update(self.offer_id.as_bytes());
        h.update(&self.commitment);
        h.update(&[self.index]);
        h.update(&self.root);
        let mut id = [0; 16];
        id.copy_from_slice(&h.finalize().as_bytes()[..16]);
        id
    }

    pub fn wow_recipient_id(&self) -> [u8; 16] {
        *self.receiver.device_id.as_uuid().as_bytes()
    }

    /// Exact canonical binary bytes. Unknown fields and variable-size
    /// encodings are impossible; no attacker-controlled allocations.
    pub fn encode(&self) -> [u8; NOTICE_BYTES] {
        let mut out = [0u8; NOTICE_BYTES];
        let mut at = 0usize;
        fn field(out: &mut [u8; NOTICE_BYTES], at: &mut usize, bytes: &[u8]) {
            let end = *at + bytes.len();
            out[*at..end].copy_from_slice(bytes);
            *at = end;
        }
        field(&mut out, &mut at, &VERSION_MAGIC);
        field(&mut out, &mut at, self.offer_id.as_bytes());
        field(&mut out, &mut at, &self.commitment);
        field(&mut out, &mut at, self.sender.device_id.as_uuid().as_bytes());
        field(&mut out, &mut at, &self.sender.endpoint_id);
        field(&mut out, &mut at, self.receiver.device_id.as_uuid().as_bytes());
        field(&mut out, &mut at, &self.receiver.endpoint_id);
        field(&mut out, &mut at, &[self.index]);
        field(&mut out, &mut at, &self.size.to_be_bytes());
        field(&mut out, &mut at, &self.root);
        field(&mut out, &mut at, &self.expires_unix.to_be_bytes());
        debug_assert_eq!(at, NOTICE_BYTES);
        out
    }

    /// Do not decode the wire into trusted fields. Derive all fields from a
    /// locally checked Offer+Decision and demand an exact byte-for-byte match.
    /// The surrounding WOW message MUST first be verified (W3).
    pub fn decode_bound(
        bytes: &[u8],
        offer: &OfferV1,
        decision: &DecisionV1,
        authenticated_sender: PeerBinding,
        enrolled_receiver: PeerBinding,
        now: u64,
    ) -> Result<Self, AttachmentError> {
        if bytes.len() != NOTICE_BYTES || bytes[..8] != VERSION_MAGIC {
            return Err(AttachmentError::WrongNotice);
        }
        const INDEX_OFFSET: usize = 152;
        let result = Self::from_accepted(
            offer, decision, authenticated_sender, enrolled_receiver,
            bytes[INDEX_OFFSET], now
        )?;
        if bytes != result.encode() {
            return Err(AttachmentError::WrongNotice);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p2p_offer_lab::{DecisionAction, OfferedFile, SCHEMA};
    use crate::sync::DeviceId;

    const NOW: u64 = 1_800_000_000;
    fn peer(uuid: u128, key: u8) -> PeerBinding {
        PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(uuid)),
            endpoint_id: [key; 32],
        }
    }
    fn accepted() -> (OfferV1, DecisionV1, PeerBinding, PeerBinding) {
        let sender = peer(17, 11);
        let receiver = peer(23, 27);
        let offer = OfferV1 {
            schema: SCHEMA,
            offer_id: Uuid::from_u128(42),
            nonce: [91; 16],
            sender, receiver,
            created_unix: NOW, expires_unix: NOW + 120,
            files: vec![
                OfferedFile { label: "first.bin".into(), bytes: 101, blake3_root: [1; 32] },
                OfferedFile { label: "second.bin".into(), bytes: 102, blake3_root: [2; 32] },
            ],
        };
        let choice = offer.receiver_decision(
            DecisionAction::Accept, vec![1], NOW, sender, receiver
        ).unwrap();
        (offer, choice, sender, receiver)
    }

    #[test]
    fn selected_only_notice_is_fixed_bounded_and_replay_stable() {
        let (offer, decision, sender, receiver) = accepted();
        let n = AttachmentNoticeV1::from_accepted(
            &offer, &decision, sender, receiver, 1, NOW + 1
        ).unwrap();
        assert_eq!(n.encode().len(), NOTICE_BYTES);
        assert_eq!(n.size(), 102);
        assert_eq!(n.root(), [2; 32]);
        assert_eq!(n.wow_recipient_id(), *receiver.device_id.as_uuid().as_bytes());
        assert_eq!(n.message_id(), n.message_id());
        assert_eq!(AttachmentNoticeV1::decode_bound(
            &n.encode(), &offer, &decision, sender, receiver, NOW + 1
        ), Ok(n));
        assert_eq!(AttachmentNoticeV1::from_accepted(
            &offer, &decision, sender, receiver, 0, NOW + 1
        ), Err(AttachmentError::NotSelected));
    }

    #[test]
    fn copied_or_forged_bytes_and_wrong_enrollment_are_rejected() {
        let (offer, decision, sender, receiver) = accepted();
        let good = AttachmentNoticeV1::from_accepted(
            &offer, &decision, sender, receiver, 1, NOW
        ).unwrap();
        for at in [0, 8, 24, 60, 85, 120, 152, 153, 171, 194] {
            let mut tampered = good.encode();
            tampered[at] ^= 0x01;
            assert!(AttachmentNoticeV1::decode_bound(
                &tampered, &offer, &decision, sender, receiver, NOW + 1
            ).is_err(), "byte {at} altered");
        }
        assert!(AttachmentNoticeV1::decode_bound(
            &good.encode()[..NOTICE_BYTES-1], &offer, &decision, sender, receiver, NOW
        ).is_err());
        assert!(AttachmentNoticeV1::decode_bound(
            &good.encode(), &offer, &decision, peer(99,11), receiver, NOW
        ).is_err());
        assert!(AttachmentNoticeV1::decode_bound(
            &good.encode(), &offer, &decision, sender, receiver, NOW + 120
        ).is_err());
        let mut different_offer = offer.clone();
        different_offer.offer_id = Uuid::from_u128(1337);
        assert!(AttachmentNoticeV1::decode_bound(
            &good.encode(), &different_offer, &decision, sender, receiver, NOW
        ).is_err());
    }

    #[test]
    fn declined_and_out_of_bounds_never_create_notice() {
        let (offer, _, sender, receiver) = accepted();
        let decline = offer.receiver_decision(
            DecisionAction::Decline, vec![], NOW, sender, receiver
        ).unwrap();
        assert_eq!(AttachmentNoticeV1::from_accepted(
            &offer, &decline, sender, receiver, 1, NOW
        ), Err(AttachmentError::NotSelected));
        let accept = offer.receiver_decision(
            DecisionAction::Accept, vec![1], NOW, sender, receiver
        ).unwrap();
        assert_eq!(AttachmentNoticeV1::from_accepted(
            &offer, &accept, sender, receiver, 200, NOW
        ), Err(AttachmentError::InvalidIndex));
    }
}
