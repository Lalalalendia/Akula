//! Opt-in fixed-size Ed25519 sender-owner read grant for one exact
//! receiver-selected attachment and bounded byte range.
//!
//! A WOW W3 encrypted NOTICE is never a file read permission. Signing is
//! deliberately gated by a separately-supplied local object ACL evaluator
//! and an already provisioned private owner key. This module neither opens a
//! source file nor creates network request handlers. The issuer policy must
//! be implemented by the genuine Akula Core authority; a test stub is NOT
//! evidence that the installed app has source ownership checks.
//!
//! On verification, a trusted owner public key/epoch and the receiver's
//! *actual authenticated QUIC endpoint ID* MUST come from separate
//! enrollment and live transport, never from the grant or notice itself.
//! This is a grant-verification lab, NOT a production key-enrollment,
//! revocation, replay-ledger, or file-serving implementation.
#![forbid(unsafe_code)]

use crate::p2p_attachment_contract::{AttachmentNoticeV1, STAGE_BLOCK_BYTES};
use crate::p2p_offer_lab::PeerBinding;
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};

const MAGIC: &[u8; 8] = b"AKGRNT01";
const DOMAIN: &[u8] = b"Akula/Core/owner-read-grant/v1\0";
const SIGNED_BYTES: usize = 264;
pub const GRANT_BYTES: usize = SIGNED_BYTES + 64;
pub const MAX_GRANT_LIFETIME_SECS: u64 = 600;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GrantError {
    Unauthorized,
    InvalidBounds,
    InvalidIdentity,
    InvalidEpoch,
    Expired,
    InvalidWire,
    InvalidSignature,
}

/// The trusted Akula Core source-access evaluator. It MUST verify that this
/// sender is entitled to expose exactly the specified immutable object/root
/// to this enrolled receiver. The default is always deny. This interface does
/// not itself implement filesystem ownership checks.
pub trait LocalReadPolicy {
    fn permit_exact_object(
        &self,
        owner: PeerBinding,
        receiver: PeerBinding,
        root: [u8; 32],
        size: u64,
    ) -> bool;
}

pub struct DenyAllReads;
impl LocalReadPolicy for DenyAllReads {
    fn permit_exact_object(
        &self, _: PeerBinding, _: PeerBinding, _: [u8; 32], _: u64
    ) -> bool { false }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GrantWindow {
    pub start: u64,
    pub end: u64,
    pub issued_unix: u64,
    pub expires_unix: u64,
    pub owner_key_epoch: u64,
    /// Supplied by a trusted CSPRNG in the caller. No zero or repeated nonce.
    pub nonce: [u8; 16],
}

/// Only constructible after signature and binding verification; possession
/// of this struct is not a filesystem handle or a Staya network GET grant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedOwnerRead {
    message_id: [u8; 16],
    root: [u8; 32],
    total: u64,
    allowed_start: u64,
    allowed_end: u64,
    owner: PeerBinding,
    receiver: PeerBinding,
}
impl VerifiedOwnerRead {
    pub fn message_id(&self) -> [u8; 16] { self.message_id }
    pub fn root(&self) -> [u8; 32] { self.root }
    pub fn size(&self) -> u64 { self.total }
    pub fn owner(&self) -> PeerBinding { self.owner }
    pub fn receiver(&self) -> PeerBinding { self.receiver }
    pub fn bounds(&self) -> (u64, u64) { (self.allowed_start, self.allowed_end) }
    pub fn permits_exact_range(&self, start: u64, end: u64) -> bool {
        start >= self.allowed_start
            && start < end
            && end <= self.allowed_end
            && end <= self.total
    }
}

fn valid_window(
    notice: &AttachmentNoticeV1,
    w: GrantWindow,
    now: u64,
) -> Result<(), GrantError> {
    if notice.root() == [0; 32]
        || notice.size() == 0
        || w.start >= w.end
        || w.end > notice.size()
        || w.start % STAGE_BLOCK_BYTES as u64 != 0
        || w.owner_key_epoch == 0
        || w.nonce == [0; 16]
    {
        return Err(GrantError::InvalidBounds);
    }
    if w.issued_unix > now
        || now >= w.expires_unix
        || w.expires_unix > notice.expires_unix()
        || w.expires_unix <= w.issued_unix
        || w.expires_unix - w.issued_unix > MAX_GRANT_LIFETIME_SECS
    {
        return Err(GrantError::Expired);
    }
    Ok(())
}

fn body(notice: &AttachmentNoticeV1, w: GrantWindow) -> [u8; SIGNED_BYTES] {
    let mut data = [0u8; SIGNED_BYTES];
    let mut i = 0usize;
    fn copy(out: &mut [u8; SIGNED_BYTES], at: &mut usize, bytes: &[u8]) {
        out[*at..*at + bytes.len()].copy_from_slice(bytes);
        *at += bytes.len();
    }
    let sender = notice.sender();
    let receiver = notice.receiver();
    copy(&mut data, &mut i, MAGIC);
    copy(&mut data, &mut i, &notice.message_id());
    copy(&mut data, &mut i, notice.offer_id().as_bytes());
    copy(&mut data, &mut i, &notice.commitment());
    copy(&mut data, &mut i, sender.device_id.as_uuid().as_bytes());
    copy(&mut data, &mut i, &sender.endpoint_id);
    copy(&mut data, &mut i, receiver.device_id.as_uuid().as_bytes());
    copy(&mut data, &mut i, &receiver.endpoint_id);
    copy(&mut data, &mut i, &notice.root());
    copy(&mut data, &mut i, &notice.size().to_be_bytes());
    copy(&mut data, &mut i, &w.start.to_be_bytes());
    copy(&mut data, &mut i, &w.end.to_be_bytes());
    copy(&mut data, &mut i, &w.issued_unix.to_be_bytes());
    copy(&mut data, &mut i, &w.expires_unix.to_be_bytes());
    copy(&mut data, &mut i, &w.owner_key_epoch.to_be_bytes());
    copy(&mut data, &mut i, &w.nonce);
    debug_assert_eq!(i, SIGNED_BYTES);
    data
}

/// The signing decision depends on a genuine local source authorization,
/// not merely a signed WOW offer. Do not pass a caller-controlled allow-all
/// predicate from remote input. Key material is never accepted from wire.
pub fn issue_owner_read_grant(
    notice: &AttachmentNoticeV1,
    owner_private_key: &Ed25519KeyPair,
    local_policy: &impl LocalReadPolicy,
    window: GrantWindow,
    now: u64,
) -> Result<[u8; GRANT_BYTES], GrantError> {
    valid_window(notice, window, now)?;
    if !local_policy.permit_exact_object(
        notice.sender(), notice.receiver(), notice.root(), notice.size()
    ) {
        return Err(GrantError::Unauthorized);
    }
    let signed = body(notice, window);
    let mut output = [0u8; GRANT_BYTES];
    output[..SIGNED_BYTES].copy_from_slice(&signed);
    let mut to_sign = Vec::with_capacity(DOMAIN.len() + SIGNED_BYTES);
    to_sign.extend_from_slice(DOMAIN);
    to_sign.extend_from_slice(&signed);
    output[SIGNED_BYTES..].copy_from_slice(owner_private_key.sign(&to_sign).as_ref());
    Ok(output)
}

/// Independently enrolled public key + epoch and actual QUIC peer identity
/// are required. A signature cannot grant a different root, recipient,
/// expired object or an out-of-range Staya GET.
pub fn verify_owner_read_grant(
    wire: &[u8],
    approved_notice: &AttachmentNoticeV1,
    trusted_owner: PeerBinding,
    independently_enrolled_owner_pubkey: &[u8; 32],
    independently_enrolled_epoch: u64,
    authenticated_quic_receiver_endpoint: [u8; 32],
    now: u64,
) -> Result<VerifiedOwnerRead, GrantError> {
    if wire.len() != GRANT_BYTES || &wire[..8] != MAGIC {
        return Err(GrantError::InvalidWire);
    }
    if approved_notice.sender() != trusted_owner
        || approved_notice.receiver().endpoint_id != authenticated_quic_receiver_endpoint
    {
        return Err(GrantError::InvalidIdentity);
    }
    let u64_at = |at: usize| -> u64 {
        u64::from_be_bytes(wire[at..at + 8].try_into().expect("length validated"))
    };
    let w = GrantWindow {
        start: u64_at(208),
        end: u64_at(216),
        issued_unix: u64_at(224),
        expires_unix: u64_at(232),
        owner_key_epoch: u64_at(240),
        nonce: wire[248..264].try_into().expect("length validated"),
    };
    valid_window(approved_notice, w, now)?;
    if independently_enrolled_epoch == 0 || w.owner_key_epoch != independently_enrolled_epoch {
        return Err(GrantError::InvalidEpoch);
    }
    let expected = body(approved_notice, w);
    if wire[..SIGNED_BYTES] != expected {
        return Err(GrantError::InvalidWire);
    }
    let mut to_verify = Vec::with_capacity(DOMAIN.len() + SIGNED_BYTES);
    to_verify.extend_from_slice(DOMAIN);
    to_verify.extend_from_slice(&expected);
    UnparsedPublicKey::new(&ED25519, independently_enrolled_owner_pubkey)
        .verify(&to_verify, &wire[SIGNED_BYTES..])
        .map_err(|_| GrantError::InvalidSignature)?;
    Ok(VerifiedOwnerRead {
        message_id: approved_notice.message_id(),
        root: approved_notice.root(),
        total: approved_notice.size(),
        allowed_start: w.start,
        allowed_end: w.end,
        owner: approved_notice.sender(),
        receiver: approved_notice.receiver(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        p2p_attachment_contract::AttachmentNoticeV1,
        p2p_offer_lab::{DecisionAction, OfferedFile, OfferV1, PeerBinding, SCHEMA},
        sync::DeviceId,
    };
    use uuid::Uuid;

    struct ExplicitPolicy { allowed: bool }
    impl LocalReadPolicy for ExplicitPolicy {
        fn permit_exact_object(
            &self, _: PeerBinding, _: PeerBinding, _: [u8; 32], _: u64
        ) -> bool { self.allowed }
    }
    fn fixture() -> AttachmentNoticeV1 {
        let owner = PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(11)), endpoint_id: [7;32],
        };
        let receiver = PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(12)), endpoint_id: [8;32],
        };
        let offer = OfferV1 {
            schema: SCHEMA, offer_id: Uuid::from_u128(42), nonce: [3;16],
            sender: owner, receiver, created_unix: 100, expires_unix: 500,
            files: vec![OfferedFile {
                label: "photo.bin".into(), bytes: 5*1024*1024,
                blake3_root: [21;32],
            }],
        };
        let decision = offer.receiver_decision(
            DecisionAction::Accept, vec![0], 101, owner, receiver
        ).unwrap();
        AttachmentNoticeV1::from_accepted(
            &offer, &decision, owner, receiver, 0, 103
        ).unwrap()
    }
    fn window() -> GrantWindow {
        GrantWindow {
            start: 2*1024*1024,
            end: 5*1024*1024,
            issued_unix: 110,
            expires_unix: 200,
            owner_key_epoch: 7,
            nonce: [43;16],
        }
    }

    #[test]
    fn denies_by_default_and_bound_to_authorized_selected_range() {
        let n = fixture();
        let owner = Ed25519KeyPair::from_seed_unchecked(&[41;32]).unwrap();
        assert_eq!(
            issue_owner_read_grant(&n, &owner, &DenyAllReads, window(), 111),
            Err(GrantError::Unauthorized),
        );
        let grant = issue_owner_read_grant(
            &n, &owner, &ExplicitPolicy { allowed: true }, window(), 111
        ).unwrap();
        assert_eq!(grant.len(), GRANT_BYTES);
        let pk: [u8; 32] = owner.public_key().as_ref().try_into().unwrap();
        let verified = verify_owner_read_grant(
            &grant, &n, n.sender(), &pk, 7, n.receiver().endpoint_id, 112
        ).unwrap();
        assert_eq!(verified.root(), n.root());
        assert_eq!(verified.bounds(), (2*1024*1024, 5*1024*1024));
        assert!(verified.permits_exact_range(2*1024*1024, 5*1024*1024));
        assert!(!verified.permits_exact_range(0, 5*1024*1024));
        assert!(!verified.permits_exact_range(2*1024*1024, 5*1024*1024+1));
        assert!(!verified.permits_exact_range(5*1024*1024, 5*1024*1024));
    }

    #[test]
    fn modified_identity_epoch_range_expiry_signature_or_nonce_refused() {
        let n = fixture();
        let owner = Ed25519KeyPair::from_seed_unchecked(&[41;32]).unwrap();
        let pk: [u8;32] = owner.public_key().as_ref().try_into().unwrap();
        let grant = issue_owner_read_grant(
            &n, &owner, &ExplicitPolicy { allowed: true }, window(), 111
        ).unwrap();
        for at in [0, 8, 27, 70, 95, 143, 175, 207, 208, 216, 224, 232, 240, 248, 263, 264, 327] {
            let mut modified = grant;
            modified[at] ^= 1;
            assert!(
                verify_owner_read_grant(
                    &modified, &n, n.sender(), &pk, 7, n.receiver().endpoint_id, 112
                ).is_err(),
                "tampered grant byte {at} must fail",
            );
        }
        assert_eq!(verify_owner_read_grant(
            &grant, &n, n.sender(), &pk, 8, n.receiver().endpoint_id, 112
        ), Err(GrantError::InvalidEpoch));
        assert_eq!(verify_owner_read_grant(
            &grant, &n, n.sender(), &pk, 7, [77;32], 112
        ), Err(GrantError::InvalidIdentity));
        assert_eq!(verify_owner_read_grant(
            &grant, &n, n.sender(), &pk, 7, n.receiver().endpoint_id, 200
        ), Err(GrantError::Expired));
        let wrong = Ed25519KeyPair::from_seed_unchecked(&[99;32]).unwrap();
        let wrong_pk: [u8;32] = wrong.public_key().as_ref().try_into().unwrap();
        assert_eq!(verify_owner_read_grant(
            &grant, &n, n.sender(), &wrong_pk, 7, n.receiver().endpoint_id, 112
        ), Err(GrantError::InvalidSignature));
    }

    #[test]
    fn grant_issuer_refuses_unaligned_or_expired_or_blank_nonce() {
        let n = fixture();
        let owner = Ed25519KeyPair::from_seed_unchecked(&[41;32]).unwrap();
        for bad in [
            GrantWindow { start: 1, ..window() },
            GrantWindow { start: n.size(), ..window() },
            GrantWindow { end: n.size()+1, ..window() },
            GrantWindow { nonce: [0;16], ..window() },
            GrantWindow { owner_key_epoch: 0, ..window() },
            GrantWindow { expires_unix: 101, ..window() },
            GrantWindow { issued_unix: 201, ..window() },
        ] {
            assert!(issue_owner_read_grant(
                &n, &owner, &ExplicitPolicy { allowed: true }, bad, 111
            ).is_err());
        }
    }
}
