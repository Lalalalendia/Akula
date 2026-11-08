//! QA-only AKP2RCP2 issue path. Not connected to the installed UI or QUIC.
//! A caller MUST supply authenticated live peers, a trusted user-generated
//! Accept decision, and bytes/digest read from a pinned, locally rehashed file.
//! A DecisionV1 from JSON alone is never evidence of a human UI action.
//! This signer supplies neither OS-owned durable consent nor anti-rollback.
use super::{
    receipt_id_v2, verify_cold_receipt_v2, ProofError, BODY_BYTES, MAGIC, RECORD_BYTES,
    SIGNATURE_BYTES, VERSION,
};
use crate::p2p_device_identity::{self, peer_pin, DeviceIdentity, IdentityError};
use crate::p2p_offer_lab::receipt_binding::{
    match_accepted_local_receipt, BindingError, ObservedLocalReceipt,
};
use crate::p2p_offer_lab::{
    DecisionAction, DecisionV1, OfferError, OfferSession, OfferV1, PeerBinding,
};
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};

pub const SENDER_HALF_BYTES: usize = BODY_BYTES + SIGNATURE_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IssueError {
    Offer(OfferError),
    Binding(BindingError),
    Identity(IdentityError),
    Proof(ProofError),
    WrongKey,
    WrongReceipt,
    InvalidDecision,
    BadLength,
    BadTranscript,
    BadSenderSignature,
    BadSigningKey,
    InvalidTime,
}

fn put<const N: usize>(out: &mut [u8; BODY_BYTES], at: &mut usize, data: &[u8; N]) {
    out[*at..*at + N].copy_from_slice(data);
    *at += N;
}

fn make_body(
    offer: &OfferV1,
    index: u8,
    sender: PeerBinding,
    receiver: PeerBinding,
    observed: ObservedLocalReceipt,
    accepted_at: u64,
) -> Result<[u8; BODY_BYTES], IssueError> {
    offer.validate(accepted_at).map_err(IssueError::Offer)?;
    if offer.sender != sender || offer.receiver != receiver {
        return Err(IssueError::BadTranscript);
    }
    let file = offer
        .files
        .get(usize::from(index))
        .ok_or(IssueError::InvalidDecision)?;
    let expected_id = receipt_id_v2(offer, index).map_err(IssueError::Proof)?;
    if observed.receipt_id != expected_id
        || observed.sender_endpoint != sender.endpoint_id
        || observed.receiver_endpoint != receiver.endpoint_id
        || observed.bytes != file.bytes
        || observed.blake3_root != file.blake3_root
    {
        return Err(IssueError::WrongReceipt);
    }
    let mut out = [0u8; BODY_BYTES];
    let mut at = 0;
    put(&mut out, &mut at, MAGIC);
    put(&mut out, &mut at, &VERSION.to_be_bytes());
    put(&mut out, &mut at, offer.offer_id.as_bytes());
    put(&mut out, &mut at, &offer.nonce);
    put(&mut out, &mut at, &offer.commitment());
    put(&mut out, &mut at, &[index]);
    put(&mut out, &mut at, expected_id.as_bytes());
    put(&mut out, &mut at, sender.device_id.as_uuid().as_bytes());
    put(&mut out, &mut at, &sender.endpoint_id);
    put(&mut out, &mut at, receiver.device_id.as_uuid().as_bytes());
    put(&mut out, &mut at, &receiver.endpoint_id);
    put(&mut out, &mut at, &file.bytes.to_be_bytes());
    put(&mut out, &mut at, &file.blake3_root);
    put(&mut out, &mut at, &accepted_at.to_be_bytes());
    debug_assert_eq!(at, BODY_BYTES);
    Ok(out)
}

fn sign_sender_with_key(
    key: &Ed25519KeyPair,
    session: &OfferSession,
    index: u8,
    authenticated_sender: PeerBinding,
    authenticated_receiver: PeerBinding,
    rehashed_source: ObservedLocalReceipt,
    now: u64,
) -> Result<[u8; SENDER_HALF_BYTES], IssueError> {
    match_accepted_local_receipt(
        session,
        index,
        authenticated_sender,
        authenticated_receiver,
        rehashed_source,
        now,
    )
    .map_err(IssueError::Binding)?;
    if key.public_key().as_ref() != &authenticated_sender.endpoint_id[..] {
        return Err(IssueError::WrongKey);
    }
    let body = make_body(
        &session.offer,
        index,
        authenticated_sender,
        authenticated_receiver,
        rehashed_source,
        now,
    )?;
    let mut half = [0u8; SENDER_HALF_BYTES];
    half[..BODY_BYTES].copy_from_slice(&body);
    half[BODY_BYTES..].copy_from_slice(key.sign(&body).as_ref());
    Ok(half)
}

fn verify_receiver_half(
    sender_half: &[u8],
    offer: &OfferV1,
    ui_decision: &DecisionV1,
    authenticated_sender: PeerBinding,
    authenticated_receiver: PeerBinding,
    rehashed_inbox: ObservedLocalReceipt,
    now: u64,
) -> Result<(), IssueError> {
    if sender_half.len() != SENDER_HALF_BYTES {
        return Err(IssueError::BadLength);
    }
    offer.validate(now).map_err(IssueError::Offer)?;
    if ui_decision.action != DecisionAction::Accept
        || ui_decision.offer_id != offer.offer_id
        || ui_decision.manifest_commitment != offer.commitment()
    {
        return Err(IssueError::InvalidDecision);
    }
    // The signed one-byte selection sits after the domain/version, offer ID,
    // nonce and manifest commitment. The time is the final eight body bytes.
    let index = sender_half[8 + 2 + 16 + 16 + 32];
    let selected = offer
        .receiver_decision(
            DecisionAction::Accept,
            ui_decision.selected.clone(),
            now,
            authenticated_sender,
            authenticated_receiver,
        )
        .map_err(IssueError::Offer)?;
    if selected != *ui_decision || !selected.selected.contains(&index) {
        return Err(IssueError::InvalidDecision);
    }
    let accepted_at = u64::from_be_bytes(
        sender_half[BODY_BYTES - 8..BODY_BYTES]
            .try_into()
            .map_err(|_| IssueError::BadLength)?,
    );
    if accepted_at > now {
        return Err(IssueError::InvalidTime);
    }
    let expected = make_body(
        offer,
        index,
        authenticated_sender,
        authenticated_receiver,
        rehashed_inbox,
        accepted_at,
    )?;
    if &sender_half[..BODY_BYTES] != expected.as_slice() {
        return Err(IssueError::BadTranscript);
    }
    UnparsedPublicKey::new(&ED25519, &authenticated_sender.endpoint_id)
        .verify(
            &sender_half[..BODY_BYTES],
            &sender_half[BODY_BYTES..SENDER_HALF_BYTES],
        )
        .map_err(|_| IssueError::BadSenderSignature)?;
    Ok(())
}

fn countersign_with_key(
    key: &Ed25519KeyPair,
    sender_half: &[u8],
    offer: &OfferV1,
    ui_decision: &DecisionV1,
    authenticated_sender: PeerBinding,
    authenticated_receiver: PeerBinding,
    rehashed_inbox: ObservedLocalReceipt,
    now: u64,
) -> Result<[u8; RECORD_BYTES], IssueError> {
    verify_receiver_half(
        sender_half,
        offer,
        ui_decision,
        authenticated_sender,
        authenticated_receiver,
        rehashed_inbox,
        now,
    )?;
    if key.public_key().as_ref() != &authenticated_receiver.endpoint_id[..] {
        return Err(IssueError::WrongKey);
    }
    let mut record = [0u8; RECORD_BYTES];
    record[..SENDER_HALF_BYTES].copy_from_slice(sender_half);
    record[SENDER_HALF_BYTES..].copy_from_slice(key.sign(sender_half).as_ref());
    // Before exporting any bytes, make the existing independent cold verifier
    // accept the exact serialized record and independently supplied file facts.
    verify_cold_receipt_v2(
        &record,
        offer,
        authenticated_sender,
        authenticated_receiver,
        rehashed_inbox,
        now,
    )
    .map_err(IssueError::Proof)?;
    Ok(record)
}

fn load_checked_local(
    local: PeerBinding,
    selected_remote_id: &str,
    remote: PeerBinding,
) -> Result<DeviceIdentity, IssueError> {
    let identity = p2p_device_identity::load().map_err(IssueError::Identity)?;
    if identity.endpoint_public_bytes() != local.endpoint_id {
        return Err(IssueError::WrongKey);
    }
    // The selected full ID is obtained independently from the UI and the
    // remote bytes MUST come from authenticated QUIC, never from wire JSON.
    peer_pin::require_pinned_remote(selected_remote_id, &remote.endpoint_id)
        .map_err(IssueError::Identity)?;
    Ok(identity)
}

/// Construct the signing key only inside the protected seed callback. Only
/// public signature bytes, never the key object, may outlive this scope.
fn with_loaded_local_key<T>(
    local: PeerBinding,
    selected_remote_id: &str,
    remote: PeerBinding,
    sign: impl FnOnce(&Ed25519KeyPair) -> Result<T, IssueError>,
) -> Result<T, IssueError> {
    let identity = load_checked_local(local, selected_remote_id, remote)?;
    identity.with_secret_seed(|seed| {
        let key =
            Ed25519KeyPair::from_seed_unchecked(seed).map_err(|_| IssueError::BadSigningKey)?;
        sign(&key)
    })
}

/// Sign only an already accepted, single-file selection. The source bytes must
/// be rehashed through a pinned sender handle by the caller. This Windows-only
/// OS credential and peer-pin check is NOT a proof of actual receiver UI consent.
/// No credentials are silently initialized and no seed is exported.
pub fn sign_sender_half_v2(
    session: &OfferSession,
    index: u8,
    authenticated_sender: PeerBinding,
    authenticated_receiver: PeerBinding,
    selected_receiver_full_id: &str,
    rehashed_source: ObservedLocalReceipt,
    now: u64,
) -> Result<[u8; SENDER_HALF_BYTES], IssueError> {
    // Reject pending, declined, unselected and forged file facts before
    // touching the user's OS credential store.
    match_accepted_local_receipt(
        session,
        index,
        authenticated_sender,
        authenticated_receiver,
        rehashed_source,
        now,
    )
    .map_err(IssueError::Binding)?;
    with_loaded_local_key(
        authenticated_sender,
        selected_receiver_full_id,
        authenticated_receiver,
        |key| {
            sign_sender_with_key(
                key,
                session,
                index,
                authenticated_sender,
                authenticated_receiver,
                rehashed_source,
                now,
            )
        },
    )
}

/// Recipient countersigns a sender half ONLY after a fresh, pinned Inbox file
/// rehash and a trusted locally generated Accept action supplied by the native
/// UI. Arbitrary decoded DecisionV1 JSON cannot prove that action occurred.
/// Requires an independently authenticated live QUIC sender and current peer
/// pin; release and File Gate remain disconnected.
pub fn countersign_received_v2(
    sender_half: &[u8],
    offer: &OfferV1,
    trusted_ui_accept: &DecisionV1,
    authenticated_sender: PeerBinding,
    authenticated_receiver: PeerBinding,
    selected_sender_full_id: &str,
    rehashed_inbox: ObservedLocalReceipt,
    now: u64,
) -> Result<[u8; RECORD_BYTES], IssueError> {
    // Invalid wire halves and caller-supplied decisions cannot even cause
    // an OS credential lookup. This does not authenticate the UI action.
    verify_receiver_half(
        sender_half,
        offer,
        trusted_ui_accept,
        authenticated_sender,
        authenticated_receiver,
        rehashed_inbox,
        now,
    )?;
    with_loaded_local_key(
        authenticated_receiver,
        selected_sender_full_id,
        authenticated_sender,
        |key| {
            countersign_with_key(
                key,
                sender_half,
                offer,
                trusted_ui_accept,
                authenticated_sender,
                authenticated_receiver,
                rehashed_inbox,
                now,
            )
        },
    )
}

#[cfg(test)]
mod tests;
