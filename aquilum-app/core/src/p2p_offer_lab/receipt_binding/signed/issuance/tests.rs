use super::*;
use crate::p2p_offer_lab::{OfferedFile, SCHEMA};
use crate::sync::DeviceId;
use uuid::Uuid;

const NOW: u64 = 1_800_000_000;

struct Fixture {
    offer: OfferV1,
    session: OfferSession,
    accept: DecisionV1,
    sender: Ed25519KeyPair,
    receiver: Ed25519KeyPair,
    observed: ObservedLocalReceipt,
}

fn fixture() -> Fixture {
    let sender = Ed25519KeyPair::from_seed_unchecked(&[21; 32]).unwrap();
    let receiver = Ed25519KeyPair::from_seed_unchecked(&[22; 32]).unwrap();
    let mut sender_public = [0u8; 32];
    let mut receiver_public = [0u8; 32];
    sender_public.copy_from_slice(sender.public_key().as_ref());
    receiver_public.copy_from_slice(receiver.public_key().as_ref());
    let offer = OfferV1 {
        schema: SCHEMA,
        offer_id: Uuid::from_u128(101),
        nonce: [7; 16],
        sender: PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(1)),
            endpoint_id: sender_public,
        },
        receiver: PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(2)),
            endpoint_id: receiver_public,
        },
        created_unix: NOW,
        expires_unix: NOW + 120,
        files: vec![OfferedFile {
            label: "trusted-view-only.txt".into(),
            bytes: 5,
            blake3_root: *blake3::hash(b"alpha").as_bytes(),
        }],
    };
    let accept = offer
        .receiver_decision(
            DecisionAction::Accept,
            vec![0],
            NOW,
            offer.sender,
            offer.receiver,
        )
        .unwrap();
    let mut session =
        OfferSession::new_sender(offer.clone(), NOW, offer.sender, offer.receiver).unwrap();
    session.resolve(&accept, offer.receiver, NOW + 1).unwrap();
    let observed = ObservedLocalReceipt {
        receipt_id: receipt_id_v2(&offer, 0).unwrap(),
        sender_endpoint: offer.sender.endpoint_id,
        receiver_endpoint: offer.receiver.endpoint_id,
        bytes: 5,
        blake3_root: *blake3::hash(b"alpha").as_bytes(),
    };
    Fixture {
        offer,
        session,
        accept,
        sender,
        receiver,
        observed,
    }
}

fn sender_half(f: &Fixture) -> [u8; SENDER_HALF_BYTES] {
    sign_sender_with_key(
        &f.sender,
        &f.session,
        0,
        f.offer.sender,
        f.offer.receiver,
        f.observed,
        NOW + 2,
    )
    .unwrap()
}

fn finish(f: &Fixture, half: &[u8]) -> Result<[u8; RECORD_BYTES], IssueError> {
    countersign_with_key(
        &f.receiver,
        half,
        &f.offer,
        &f.accept,
        f.offer.sender,
        f.offer.receiver,
        f.observed,
        NOW + 3,
    )
}

#[test]
fn genuine_two_key_half_to_cold_v2_roundtrip() {
    let f = fixture();
    let half = sender_half(&f);
    let completed = finish(&f, &half).unwrap();
    verify_cold_receipt_v2(
        &completed,
        &f.offer,
        f.offer.sender,
        f.offer.receiver,
        f.observed,
        NOW + 200,
    )
    .unwrap();
}

#[test]
fn pending_declined_and_unselected_session_cannot_sign() {
    let f = fixture();
    let pending =
        OfferSession::new_sender(f.offer.clone(), NOW, f.offer.sender, f.offer.receiver).unwrap();
    assert!(matches!(
        sign_sender_with_key(
            &f.sender,
            &pending,
            0,
            f.offer.sender,
            f.offer.receiver,
            f.observed,
            NOW + 2,
        ),
        Err(IssueError::Binding(BindingError::NotAccepted))
    ));
    assert!(matches!(
        sign_sender_with_key(
            &f.sender,
            &f.session,
            1,
            f.offer.sender,
            f.offer.receiver,
            f.observed,
            NOW + 2,
        ),
        Err(IssueError::Binding(BindingError::UnselectedFile))
    ));
    let mut declined =
        OfferSession::new_sender(f.offer.clone(), NOW, f.offer.sender, f.offer.receiver).unwrap();
    let decline = f
        .offer
        .receiver_decision(
            DecisionAction::Decline,
            vec![],
            NOW,
            f.offer.sender,
            f.offer.receiver,
        )
        .unwrap();
    declined.resolve(&decline, f.offer.receiver, NOW + 1).unwrap();
    assert!(matches!(
        sign_sender_with_key(
            &f.sender,
            &declined,
            0,
            f.offer.sender,
            f.offer.receiver,
            f.observed,
            NOW + 2,
        ),
        Err(IssueError::Binding(BindingError::NotAccepted))
    ));
    let mut wrong = f.observed;
    wrong.blake3_root[0] ^= 1;
    assert!(matches!(
        sign_sender_with_key(
            &f.sender,
            &f.session,
            0,
            f.offer.sender,
            f.offer.receiver,
            wrong,
            NOW + 2,
        ),
        Err(IssueError::Binding(BindingError::ReceiptMismatch))
    ));
}

#[test]
fn sender_half_mutation_wrong_key_and_untrusted_decision_fail() {
    let f = fixture();
    assert!(matches!(
        sign_sender_with_key(
            &f.receiver,
            &f.session,
            0,
            f.offer.sender,
            f.offer.receiver,
            f.observed,
            NOW + 2,
        ),
        Err(IssueError::WrongKey)
    ));
    let half = sender_half(&f);
    let mut changed = half;
    changed[10] ^= 1;
    assert!(finish(&f, &changed).is_err());
    changed = half;
    changed[BODY_BYTES] ^= 1;
    assert!(matches!(
        finish(&f, &changed),
        Err(IssueError::BadSenderSignature)
    ));
    let mut decline = f.accept.clone();
    decline.action = DecisionAction::Decline;
    assert!(matches!(
        countersign_with_key(
            &f.receiver,
            &half,
            &f.offer,
            &decline,
            f.offer.sender,
            f.offer.receiver,
            f.observed,
            NOW + 3,
        ),
        Err(IssueError::InvalidDecision)
    ));
}

#[test]
fn wrong_peer_receipt_and_expired_acceptance_fail_closed() {
    let f = fixture();
    let half = sender_half(&f);
    let mut wrong = f.observed;
    wrong.receipt_id = Uuid::from_u128(987);
    assert!(matches!(
        countersign_with_key(
            &f.receiver,
            &half,
            &f.offer,
            &f.accept,
            f.offer.sender,
            f.offer.receiver,
            wrong,
            NOW + 3,
        ),
        Err(IssueError::WrongReceipt)
    ));
    let mut substituted = f.offer.receiver;
    substituted.endpoint_id = [42; 32];
    assert!(finish(&f, &half[..SENDER_HALF_BYTES - 1]).is_err());
    assert!(countersign_with_key(
        &f.receiver, &half, &f.offer, &f.accept, f.offer.sender,
        substituted, f.observed, NOW + 3,
    )
    .is_err());
    assert!(matches!(
        finish_at(&f, &half, NOW + 200),
        Err(IssueError::Offer(OfferError::Expired))
    ));
}

fn finish_at(
    f: &Fixture,
    half: &[u8],
    now: u64,
) -> Result<[u8; RECORD_BYTES], IssueError> {
    countersign_with_key(
        &f.receiver,
        half,
        &f.offer,
        &f.accept,
        f.offer.sender,
        f.offer.receiver,
        f.observed,
        now,
    )
}

#[cfg(not(windows))]
#[test]
fn absent_os_credential_backend_never_falls_back_to_test_key() {
    let f = fixture();
    assert!(matches!(
        sign_sender_half_v2(
            &f.session,
            0,
            f.offer.sender,
            f.offer.receiver,
            "not-a-valid-pin",
            f.observed,
            NOW + 2
        ),
        Err(IssueError::Identity(IdentityError::UnsupportedPlatform))
    ));
}
