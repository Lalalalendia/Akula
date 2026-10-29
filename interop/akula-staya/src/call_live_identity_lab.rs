//! AK-CALL-01: live authenticated Iroh peer, OS peer pin and independently
//! witnessed roster must agree before a passive call can be admitted.
//! This is NOT a user's microphone, camera or screen permission. No media,
//! socket setup, Vault access, data recording or background reconnect lives here.
//!
//! LocalPeerPolicy and trusted_local are supplied by a separately reviewed host.
//! In particular, a roster entry with RecordedOnly status is NOT permission
//! for the remote peer to call. The host must separately authorize the contact.

use aquilum_core::{
    call_signaling_lab::{CallError, CallSession, LocalPeerPolicy, RemoteEvent},
    p2p_offer_lab::PeerBinding,
    sync::{DeviceId, VaultId, device_roster_lab::VerifiedRosterObservation},
};
use iroh::endpoint::Connection;

use crate::roster_observation_lab::observe_live_pinned_roster_peer;

/// Create the callee state only when (1) QUIC/TLS authenticated the remote
/// EndpointId, (2) Windows OS storage currently confirms the selected pin,
/// (3) an independently owner-witnessed roster binds this ID to DeviceId,
/// and (4) the local host allowed the call. Offer bytes are never authority.
#[allow(clippy::too_many_arguments)]
pub fn admit_incoming_call(
    connection: &Connection,
    selected_remote_endpoint: &str,
    trusted_roster: &VerifiedRosterObservation,
    expected_vault: VaultId,
    expected_remote_device: DeviceId,
    trusted_local: PeerBinding,
    peer_policy: LocalPeerPolicy,
    offer: &[u8],
    now_unix: u64,
) -> Result<CallSession, CallError> {
    let observed = observe_live_pinned_roster_peer(
        connection,
        selected_remote_endpoint,
        trusted_roster,
        expected_vault,
        expected_remote_device,
    )
    .map_err(|_| CallError::Denied)?;
    CallSession::incoming(
        offer,
        observed.metadata_peer_binding(),
        trusted_local,
        peer_policy,
        now_unix,
    )
}

/// Re-read OS pin and current independently verified roster on EVERY remote
/// call operation. Failure terminally closes the passive call state. The host
/// must independently stop OS media capture when this fails.
#[allow(clippy::too_many_arguments)]
pub fn receive_live_call(
    session: &mut CallSession,
    connection: &Connection,
    selected_remote_endpoint: &str,
    trusted_roster: &VerifiedRosterObservation,
    expected_vault: VaultId,
    expected_remote_device: DeviceId,
    peer_policy: LocalPeerPolicy,
    control: &[u8],
    now_unix: u64,
) -> Result<RemoteEvent, CallError> {
    let observed = match observe_live_pinned_roster_peer(
        connection,
        selected_remote_endpoint,
        trusted_roster,
        expected_vault,
        expected_remote_device,
    ) {
        Ok(observed) => observed,
        Err(_) => {
            session.revoke();
            return Err(CallError::Revoked);
        }
    };
    session.receive(
        control,
        observed.metadata_peer_binding(),
        peer_policy,
        now_unix,
    )
}
