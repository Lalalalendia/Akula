//! NODE-LIVE-ID-01: bind an established Iroh QUIC peer to an owner-signed
//! roster AND a separate, exact-revision owner read grant.
//!
//! This is a non-shipping adapter over existing Akula modules. It never
//! enrolls devices, trusts a ticket/offer-supplied endpoint, writes a Vault,
//! issues rights, opens a listener or changes the Staya provider policy.
//! The caller must supply independently protected CURRENT owner/epoch pins
//! and a real single-use challenge fence. The lab supplies these manually.

use aquilum_core::node_replica_auth_lab::{
    DeviceReadProof, IndependentReadTrust, OwnerReadGrant, read_with_device_proof,
    signed_grant_digest,
};
use aquilum_core::sync::device_roster_lab::{
    IndependentlyPinnedOwner, OwnerSignedRoster, RosterStatus, inspect_signed_roster,
};
use iroh::endpoint::Connection;
use std::path::Path;

/// Must be backed by a *trusted* server/session store in production.
/// A freshly allocated permissive implementation is NOT a replay fence.
pub trait OneUseChallengeFence {
    /// Atomically consume this exact grant/challenge/QUIC-peer tuple once.
    fn consume_once(&mut self, peer: [u8; 32], grant_digest: [u8; 32], challenge: [u8; 32])
    -> bool;
}

/// All failures have the same public shape, with no metadata disclosure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeReadDenied;

/// The ONLY peer identity used here is obtained from the completed TLS/QUIC
/// connection, not from invitation data, claimed DeviceId, or a test CLI.
///
/// Both host-controlled witnesses must be current, exact and independently
/// trusted. This function's success is a bounded lab read, NOT a production
/// network GET or proof of a durable server-side identity/key store.
pub fn read_from_live_iroh_peer(
    connection: &Connection,
    roster: &OwnerSignedRoster,
    roster_pin: &IndependentlyPinnedOwner,
    grant: &OwnerReadGrant,
    grant_pin: &IndependentReadTrust,
    device_proof: &DeviceReadProof,
    fence: &mut impl OneUseChallengeFence,
    node_state: &Path,
) -> Result<Vec<u8>, NodeReadDenied> {
    // A lower bound is not enough for live access: enforce an exact separately
    // witnessed roster epoch/digest before even considering membership.
    let exact_digest = roster.claim_digest().map_err(|_| NodeReadDenied)?;
    if roster_pin.witnessed_min_epoch != roster.epoch
        || roster_pin.witnessed_epoch_digest != Some(exact_digest)
        || roster_pin.owner_public_key != grant_pin.trusted_owner_public_key
        || roster_pin.vault_id != grant.vault_id
        || roster_pin.vault_id != grant_pin.trusted_vault_id
        || roster.epoch != grant.grant_epoch
        || roster.epoch != grant_pin.witnessed_current_epoch
    {
        return Err(NodeReadDenied);
    }

    let observed = inspect_signed_roster(roster, roster_pin).map_err(|_| NodeReadDenied)?;
    // Iroh's Connection::remote_id is only available after the authenticated
    // handshake; unlike an offered EndpointId it is not peer-controlled text.
    let authenticated = *connection.remote_id().as_bytes();
    let mut matches = observed
        .devices()
        .iter()
        .filter(|entry| entry.endpoint_public_key() == authenticated);
    let device = matches.next().ok_or(NodeReadDenied)?;
    if matches.next().is_some()
        || device.status() != RosterStatus::RecordedOnly
        || device.device_public_key() != authenticated
        || device.device_id() != grant.device_id
        || grant.device_public_key != authenticated
        || observed.vault_id() != grant.vault_id
        || observed.epoch() != grant.grant_epoch
    {
        return Err(NodeReadDenied);
    }

    // Same *existing Iroh key* is used for device proof. No second secret is
    // introduced. This does not give RecordedOnly a Vault read entitlement:
    // the next verifier separately checks the owner-signed exact read grant.
    let grant_digest = signed_grant_digest(grant).map_err(|_| NodeReadDenied)?;
    if !fence.consume_once(
        authenticated,
        grant_digest,
        grant_pin.current_session_challenge,
    ) {
        return Err(NodeReadDenied);
    }
    read_with_device_proof(node_state, grant, grant_pin, device_proof).map_err(|_| NodeReadDenied)
}
