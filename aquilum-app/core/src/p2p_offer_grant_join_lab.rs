//! Test-only compatibility falsifier for OfferV1 and RN-4C/#433 StoredPull.
//!
//! This file uses a synthetic snapshot of the older research ledger, not a
//! real Core-owned policy service. It never emits authorization or touches
//! user files. A positive correlation is deliberately NOT a grant.

use crate::p2p_offer_lab::{
    DecisionAction, OfferSession, OfferV1, OfferedFile, Outcome, PeerBinding, SelectionDraft,
};
use crate::sync::{DeviceId, VaultId};
use uuid::Uuid;

const MAX_RN4C_OBJECT: u64 = 4 * 1024 * 1024;
const NOW: u64 = 1_800_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Hold {
    InvalidOffer,
    WrongPeer,
    WrongVault,
    InvalidConsent,
    LegacySingleObject,
    LegacySize,
    WrongIssuer,
    StaleEpoch,
    WrongSubject,
    WrongObject,
    WrongNonce,
    WrongExpiry,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Observation {
    ConsistentOnlyNoAuthorization,
}

#[derive(Clone, Copy)]
struct StoredPullShape {
    vault_id: VaultId,
    device_id: DeviceId,
    authenticated_endpoint: [u8; 32],
    trusted_issuer: [u8; 32],
    object_blake3: [u8; 32],
    object_length: u64,
    epoch: u64,
    expires_at: u64,
    nonce: [u8; 16],
}

#[derive(Clone, Copy)]
struct LocalClaims {
    enrolled_sender: PeerBinding,
    enrolled_receiver: PeerBinding,
    live_remote_endpoint: [u8; 32],
    selected_vault: VaultId,
    expected_epoch: u64,
    trusted_issuer: [u8; 32],
}

/// Compare exactly one selected root against a simulated old grant record.
/// LocalClaims are test-injected, never verified real QUIC/OS Broker evidence.
/// There is no permission-bearing return value, including on a full match.
fn correlate(
    offer: &OfferV1,
    selection: &SelectionDraft,
    grant: StoredPullShape,
    local: LocalClaims,
    now: u64,
) -> Result<Observation, Hold> {
    offer.validate(now).map_err(|_| Hold::InvalidOffer)?;
    if offer.sender != local.enrolled_sender
        || offer.receiver != local.enrolled_receiver
        || offer.receiver.endpoint_id != local.live_remote_endpoint
    {
        return Err(Hold::WrongPeer);
    }
    if local.selected_vault.as_uuid().is_nil() || grant.vault_id != local.selected_vault {
        return Err(Hold::WrongVault);
    }
    if selection.offer_id != offer.offer_id || selection.commitment != offer.commitment() {
        return Err(Hold::InvalidConsent);
    }
    if selection.files.len() != 1 {
        return Err(Hold::LegacySingleObject);
    }
    let file = &selection.files[0];
    if !offer.files.contains(file) {
        return Err(Hold::InvalidConsent);
    }
    if file.bytes > MAX_RN4C_OBJECT {
        return Err(Hold::LegacySize);
    }
    if local.trusted_issuer == [0; 32] || grant.trusted_issuer != local.trusted_issuer {
        return Err(Hold::WrongIssuer);
    }
    if local.expected_epoch == 0 || grant.epoch != local.expected_epoch {
        return Err(Hold::StaleEpoch);
    }
    if grant.device_id != offer.receiver.device_id
        || grant.authenticated_endpoint != offer.receiver.endpoint_id
    {
        return Err(Hold::WrongSubject);
    }
    if grant.object_blake3 != file.blake3_root || grant.object_length != file.bytes {
        return Err(Hold::WrongObject);
    }
    if grant.nonce != offer.nonce {
        return Err(Hold::WrongNonce);
    }
    if grant.expires_at <= now || grant.expires_at > offer.expires_unix {
        return Err(Hold::WrongExpiry);
    }
    Ok(Observation::ConsistentOnlyNoAuthorization)
}

fn peer(id: u128, endpoint: u8) -> PeerBinding {
    PeerBinding {
        device_id: DeviceId::from_uuid(Uuid::from_u128(id)),
        endpoint_id: [endpoint; 32],
    }
}

fn fixture() -> (OfferV1, SelectionDraft, StoredPullShape, LocalClaims) {
    let offer = OfferV1 {
        schema: 1,
        offer_id: Uuid::from_u128(91),
        nonce: [21; 16],
        sender: peer(1, 31),
        receiver: peer(2, 41),
        created_unix: NOW,
        expires_unix: NOW + 120,
        files: vec![
            OfferedFile {
                label: "accepted.bin".into(),
                bytes: 1024,
                blake3_root: [11; 32],
            },
            OfferedFile {
                label: "other.bin".into(),
                bytes: 1024,
                blake3_root: [12; 32],
            },
        ],
    };
    let decision = offer
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
    let Outcome::Selected(selection) = session
        .resolve(&decision, offer.receiver, NOW + 1)
        .unwrap()
    else {
        panic!("expected selection");
    };
    let grant = StoredPullShape {
        vault_id: VaultId::from_uuid(Uuid::from_u128(3)),
        device_id: offer.receiver.device_id,
        authenticated_endpoint: offer.receiver.endpoint_id,
        trusted_issuer: [51; 32],
        object_blake3: offer.files[0].blake3_root,
        object_length: offer.files[0].bytes,
        epoch: 5,
        expires_at: NOW + 60,
        nonce: offer.nonce,
    };
    let claims = LocalClaims {
        enrolled_sender: offer.sender,
        enrolled_receiver: offer.receiver,
        live_remote_endpoint: offer.receiver.endpoint_id,
        selected_vault: grant.vault_id,
        expected_epoch: grant.epoch,
        trusted_issuer: grant.trusted_issuer,
    };
    (offer, selection, grant, claims)
}

#[test]
fn one_file_matches_but_never_yields_authority() {
    let (offer, selection, grant, local) = fixture();
    assert_eq!(
        correlate(&offer, &selection, grant, local, NOW + 2),
        Ok(Observation::ConsistentOnlyNoAuthorization),
    );
}

#[test]
fn batch_selection_fails_old_one_object_grant_model() {
    let (offer, _, grant, local) = fixture();
    let d = offer
        .receiver_decision(
            DecisionAction::Accept,
            vec![0, 1],
            NOW,
            offer.sender,
            offer.receiver,
        )
        .unwrap();
    let mut session =
        OfferSession::new_sender(offer.clone(), NOW, offer.sender, offer.receiver).unwrap();
    let Outcome::Selected(selected) = session.resolve(&d, offer.receiver, NOW + 1).unwrap()
    else {
        panic!("expected selection");
    };
    assert_eq!(
        correlate(&offer, &selected, grant, local, NOW + 2),
        Err(Hold::LegacySingleObject),
    );
}

#[test]
fn greater_than_four_megabytes_cannot_be_mapped() {
    let (mut offer, _, mut grant, local) = fixture();
    offer.files[0].bytes = MAX_RN4C_OBJECT + 1;
    grant.object_length = offer.files[0].bytes;
    offer.validate(NOW).unwrap();
    let d = offer
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
    let Outcome::Selected(selected) = session.resolve(&d, offer.receiver, NOW + 1).unwrap()
    else {
        panic!("expected selection");
    };
    assert_eq!(
        correlate(&offer, &selected, grant, local, NOW + 2),
        Err(Hold::LegacySize),
    );
}

#[test]
fn untrusted_peer_identity_is_denied() {
    let (offer, selected, grant, mut local) = fixture();
    local.live_remote_endpoint = [98; 32];
    assert_eq!(
        correlate(&offer, &selected, grant, local, NOW + 2),
        Err(Hold::WrongPeer),
    );
    local.live_remote_endpoint = offer.receiver.endpoint_id;
    local.enrolled_sender = peer(1, 98);
    assert_eq!(
        correlate(&offer, &selected, grant, local, NOW + 2),
        Err(Hold::WrongPeer),
    );
}

#[test]
fn wrong_vault_and_recipient_fail() {
    let (offer, selected, mut grant, local) = fixture();
    grant.vault_id = VaultId::new();
    assert_eq!(
        correlate(&offer, &selected, grant, local, NOW + 2),
        Err(Hold::WrongVault),
    );
    grant.vault_id = local.selected_vault;
    grant.device_id = DeviceId::new();
    assert_eq!(
        correlate(&offer, &selected, grant, local, NOW + 2),
        Err(Hold::WrongSubject),
    );
    grant.device_id = offer.receiver.device_id;
    grant.authenticated_endpoint = [98; 32];
    assert_eq!(
        correlate(&offer, &selected, grant, local, NOW + 2),
        Err(Hold::WrongSubject),
    );
}

#[test]
fn object_root_size_and_nonce_are_bound() {
    let (offer, selected, mut grant, local) = fixture();
    grant.object_blake3 = [98; 32];
    assert_eq!(
        correlate(&offer, &selected, grant, local, NOW + 2),
        Err(Hold::WrongObject),
    );
    grant.object_blake3 = offer.files[0].blake3_root;
    grant.object_length += 1;
    assert_eq!(
        correlate(&offer, &selected, grant, local, NOW + 2),
        Err(Hold::WrongObject),
    );
    grant.object_length = offer.files[0].bytes;
    grant.nonce = [98; 16];
    assert_eq!(
        correlate(&offer, &selected, grant, local, NOW + 2),
        Err(Hold::WrongNonce),
    );
}

#[test]
fn epoch_issuer_and_deadline_are_bound() {
    let (offer, selected, mut grant, mut local) = fixture();
    grant.epoch = 4;
    assert_eq!(
        correlate(&offer, &selected, grant, local, NOW + 2),
        Err(Hold::StaleEpoch),
    );
    grant.epoch = 5;
    local.trusted_issuer = [98; 32];
    assert_eq!(
        correlate(&offer, &selected, grant, local, NOW + 2),
        Err(Hold::WrongIssuer),
    );
    local.trusted_issuer = grant.trusted_issuer;
    grant.expires_at = NOW + 1;
    assert_eq!(
        correlate(&offer, &selected, grant, local, NOW + 2),
        Err(Hold::WrongExpiry),
    );
    grant.expires_at = offer.expires_unix + 1;
    assert_eq!(
        correlate(&offer, &selected, grant, local, NOW + 2),
        Err(Hold::WrongExpiry),
    );
}

#[test]
fn mutated_offer_after_accept_cannot_reuse_selection() {
    let (mut offer, selected, grant, local) = fixture();
    offer.files[0].blake3_root = [98; 32];
    assert_eq!(
        correlate(&offer, &selected, grant, local, NOW + 2),
        Err(Hold::InvalidConsent),
    );
}
