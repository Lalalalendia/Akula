//! Read-only correspondence: a live OS-pinned QUIC peer and an owner-signed Vault roster.
//!
//! This observation is NOT a Staya GET grant, File Gate authority, or durable
//! revocation witness. Callers must independently pin the roster owner and
//! epoch floor; no bytes from an Offer, ticket or peer message may supply them.
use aquilum_core::{
    p2p_offer_lab::PeerBinding,
    sync::{
        DeviceId, VaultId,
        device_roster_lab::{RosterStatus, VerifiedRosterObservation},
    },
};
use iroh::endpoint::Connection;

use crate::live_identity::require_pinned_live_peer;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationError {
    OsPinNotConfirmed,
    WrongVault,
    UnknownEndpoint,
    WrongDevice,
    Revoked,
}

/// Read-only metadata correspondence. Its fields cannot convey file privileges.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservedRosterPeer {
    peer: PeerBinding,
    vault_id: VaultId,
    roster_epoch: u64,
}

impl ObservedRosterPeer {
    /// May be used to compare Akula Offer identities. NEVER authorize GET.
    pub fn metadata_peer_binding(&self) -> PeerBinding {
        self.peer
    }

    pub fn vault_id(&self) -> VaultId {
        self.vault_id
    }

    pub fn roster_epoch(&self) -> u64 {
        self.roster_epoch
    }
}

/// This is an observation only. In particular, the verified roster's
/// RecordedOnly state is NOT a positive device or object permission.
///
/// The independent OS-persisted pin is re-read on every call, and the actual
/// endpoint bytes come from QUIC/TLS, NOT the selected string or Offer JSON.
pub fn observe_live_pinned_roster_peer(
    connection: &Connection,
    selected_full_endpoint_id: &str,
    roster: &VerifiedRosterObservation,
    expected_vault_id: VaultId,
    expected_device_id: DeviceId,
) -> Result<ObservedRosterPeer, ObservationError> {
    require_pinned_live_peer(connection, selected_full_endpoint_id)
        .map_err(|_| ObservationError::OsPinNotConfirmed)?;
    observe_verified_endpoint(
        roster,
        expected_vault_id,
        expected_device_id,
        *connection.remote_id().as_bytes(),
    )
}

/// The roster half of the gate, split out only for deterministic tests.
/// Successful correspondence is still neither proof of OS pin nor a grant.
fn observe_verified_endpoint(
    roster: &VerifiedRosterObservation,
    expected_vault_id: VaultId,
    expected_device_id: DeviceId,
    live_endpoint: [u8; 32],
) -> Result<ObservedRosterPeer, ObservationError> {
    if roster.vault_id() != expected_vault_id {
        return Err(ObservationError::WrongVault);
    }
    let device = roster
        .devices()
        .iter()
        .find(|device| device.endpoint_public_key() == live_endpoint)
        .ok_or(ObservationError::UnknownEndpoint)?;
    if device.device_id() != expected_device_id {
        return Err(ObservationError::WrongDevice);
    }
    if device.status() != RosterStatus::RecordedOnly {
        return Err(ObservationError::Revoked);
    }
    Ok(ObservedRosterPeer {
        peer: PeerBinding {
            device_id: device.device_id(),
            endpoint_id: live_endpoint,
        },
        vault_id: expected_vault_id,
        roster_epoch: roster.epoch(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aquilum_core::sync::device_roster_lab::{
        ClaimedDevice, IndependentlyPinnedOwner, OwnerSignedRoster, RosterError,
        inspect_signed_roster,
    };
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn signed_fixture(status: RosterStatus) -> (VerifiedRosterObservation, VaultId, DeviceId) {
        let owner = Ed25519KeyPair::from_seed_unchecked(&[11; 32]).unwrap();
        let vault_id = VaultId::new();
        let device_id = DeviceId::new();
        let mut roster = OwnerSignedRoster {
            vault_id,
            epoch: 2,
            devices: vec![ClaimedDevice {
                device_id,
                device_public_key: [2; 32],
                endpoint_public_key: [3; 32],
                status,
            }],
            signature: [0; 64],
        };
        roster
            .signature
            .copy_from_slice(owner.sign(&roster.canonical_claim().unwrap()).as_ref());
        let trusted_owner = IndependentlyPinnedOwner {
            vault_id,
            owner_public_key: owner.public_key().as_ref().try_into().unwrap(),
            witnessed_min_epoch: 1,
            witnessed_epoch_digest: None,
        };
        let observed = inspect_signed_roster(&roster, &trusted_owner).unwrap();
        (observed, vault_id, device_id)
    }

    #[test]
    fn an_independently_signed_roster_matches_only_the_exact_quic_endpoint() {
        let (observed, vault_id, device_id) = signed_fixture(RosterStatus::RecordedOnly);
        let peer = observe_verified_endpoint(&observed, vault_id, device_id, [3; 32]).unwrap();
        assert_eq!(peer.metadata_peer_binding().device_id, device_id);
        assert_eq!(peer.metadata_peer_binding().endpoint_id, [3; 32]);
        assert_eq!(peer.vault_id(), vault_id);
        assert_eq!(peer.roster_epoch(), 2);
        assert_eq!(
            observe_verified_endpoint(&observed, vault_id, device_id, [4; 32]),
            Err(ObservationError::UnknownEndpoint)
        );
        assert_eq!(
            observe_verified_endpoint(&observed, VaultId::new(), device_id, [3; 32]),
            Err(ObservationError::WrongVault)
        );
        assert_eq!(
            observe_verified_endpoint(&observed, vault_id, DeviceId::new(), [3; 32]),
            Err(ObservationError::WrongDevice)
        );
    }

    #[test]
    fn revoked_device_never_yields_an_offer_peer_binding() {
        let (observed, vault_id, device_id) = signed_fixture(RosterStatus::Revoked);
        assert_eq!(
            observe_verified_endpoint(&observed, vault_id, device_id, [3; 32]),
            Err(ObservationError::Revoked)
        );
    }

    #[test]
    fn a_signed_roster_still_requires_an_external_epoch_witness() {
        let owner = Ed25519KeyPair::from_seed_unchecked(&[11; 32]).unwrap();
        let vault_id = VaultId::new();
        let mut signed = OwnerSignedRoster {
            vault_id,
            epoch: 2,
            devices: vec![],
            signature: [0; 64],
        };
        signed
            .signature
            .copy_from_slice(owner.sign(&signed.canonical_claim().unwrap()).as_ref());
        let too_new = IndependentlyPinnedOwner {
            vault_id,
            owner_public_key: owner.public_key().as_ref().try_into().unwrap(),
            witnessed_min_epoch: 3,
            witnessed_epoch_digest: None,
        };
        assert_eq!(
            inspect_signed_roster(&signed, &too_new),
            Err(RosterError::Rollback)
        );
    }
}
