//! STY-DEVICE-01a: read-only owner-signed registry observation.
//!
//! Not pairing, private-key custody, remote proof-of-possession, live QUIC
//! authentication, or File Gate authorization. No installed Akula code calls
//! this opt-in laboratory. Trusted pins and epoch floors are EXTERNAL inputs.

// Scope lint failure to this newly owned module, not legacy aquilum-core.
#![deny(clippy::all)]

use std::collections::BTreeSet;

use ring::signature::{self, UnparsedPublicKey};

use super::{DeviceId, VaultId};

const DOMAIN: &[u8] = b"Akula/STY-DEVICE-01a/owner-roster/v1\0";
pub const MAX_ROSTER_DEVICES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RosterStatus {
    /// An owner signed this enrollment record, not a live identity proof.
    RecordedOnly,
    Revoked,
}

impl RosterStatus {
    fn tag(self) -> u8 {
        match self {
            Self::RecordedOnly => 1,
            Self::Revoked => 2,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimedDevice {
    pub device_id: DeviceId,
    /// Ed25519 public key of the claimed device, not a fingerprint.
    pub device_public_key: [u8; 32],
    /// Pinned Iroh endpoint public bytes; no live channel is checked here.
    pub endpoint_public_key: [u8; 32],
    pub status: RosterStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerSignedRoster {
    pub vault_id: VaultId,
    pub epoch: u64,
    /// Sorted in ascending DeviceId order; no endpoint key collisions.
    pub devices: Vec<ClaimedDevice>,
    /// Detached Ed25519 signature of canonical_claim().
    pub signature: [u8; 64],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IndependentlyPinnedOwner {
    pub vault_id: VaultId,
    pub owner_public_key: [u8; 32],
    /// Durable independent monotonic witness, never read from candidate.
    pub witnessed_min_epoch: u64,
    /// Digest of that exact epoch's claim, sealed by an independent witness.
    /// Mandatory when the candidate epoch equals witnessed_min_epoch.
    pub witnessed_epoch_digest: Option<[u8; 32]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RosterError {
    MissingIndependentPin,
    InvalidClaim,
    Oversize,
    AmbiguousIdentity,
    WrongVault,
    Rollback,
    WitnessMismatch,
    InvalidSignature,
}

impl OwnerSignedRoster {
    /// Canonical, domain-separated binary claim; integers use big endian.
    /// Reject ambiguity before signing or attempting signature verification.
    pub fn canonical_claim(&self) -> Result<Vec<u8>, RosterError> {
        if self.vault_id.as_uuid().is_nil() || self.epoch == 0 {
            return Err(RosterError::InvalidClaim);
        }
        if self.devices.len() > MAX_ROSTER_DEVICES {
            return Err(RosterError::Oversize);
        }
        let mut previous: Option<DeviceId> = None;
        let mut endpoints = BTreeSet::new();
        for device in &self.devices {
            if device.device_id.as_uuid().is_nil()
                || device.device_public_key == [0; 32]
                || device.endpoint_public_key == [0; 32]
            {
                return Err(RosterError::InvalidClaim);
            }
            if previous.is_some_and(|id| id >= device.device_id)
                || !endpoints.insert(device.endpoint_public_key)
            {
                return Err(RosterError::AmbiguousIdentity);
            }
            previous = Some(device.device_id);
        }
        let mut bytes = Vec::with_capacity(DOMAIN.len() + 26 + self.devices.len() * 81);
        bytes.extend_from_slice(DOMAIN);
        bytes.push(1); // version
        let vault = self.vault_id.as_uuid();
        bytes.extend_from_slice(vault.as_bytes());
        bytes.extend_from_slice(&self.epoch.to_be_bytes());
        bytes.push(self.devices.len() as u8); // already bounded to 32
        for device in &self.devices {
            let id = device.device_id.as_uuid();
            bytes.extend_from_slice(id.as_bytes());
            bytes.extend_from_slice(&device.device_public_key);
            bytes.extend_from_slice(&device.endpoint_public_key);
            bytes.push(device.status.tag());
        }
        Ok(bytes)
    }

    /// A domain-separated snapshot digest for independent durable witnessing.
    /// A digest supplied by the candidate itself is NOT a trusted witness.
    pub fn claim_digest(&self) -> Result<[u8; 32], RosterError> {
        let mut digest = blake3::Hasher::new_derive_key("Akula/STY-DEVICE-01a/witness/v1");
        digest.update(&self.canonical_claim()?);
        Ok(*digest.finalize().as_bytes())
    }
}

/// A verified *observation*, deliberately not a File Gate grant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservedDevice {
    id: DeviceId,
    device_public_key: [u8; 32],
    endpoint_public_key: [u8; 32],
    status: RosterStatus,
}

impl ObservedDevice {
    pub fn device_id(&self) -> DeviceId {
        self.id
    }

    pub fn status(&self) -> RosterStatus {
        self.status
    }

    pub fn device_public_key(&self) -> [u8; 32] {
        self.device_public_key
    }

    pub fn endpoint_public_key(&self) -> [u8; 32] {
        self.endpoint_public_key
    }

    /// UI hint only, not a peer authentication token.
    pub fn display_fingerprint(&self) -> [u8; 32] {
        let mut hash = blake3::Hasher::new_derive_key("Akula/STY-DEVICE-01a/device-fingerprint/v1");
        hash.update(&self.device_public_key);
        *hash.finalize().as_bytes()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedRosterObservation {
    vault_id: VaultId,
    epoch: u64,
    devices: Vec<ObservedDevice>,
}

impl VerifiedRosterObservation {
    pub fn vault_id(&self) -> VaultId {
        self.vault_id
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn devices(&self) -> &[ObservedDevice] {
        &self.devices
    }
}

/// Verifies the signature and antirollback floor. The floor and trusted owner
/// pin MUST be provided by separate OS/installer authority. Success is NEVER
/// live identity proof, peer consent, transfer entitlement or File Gate ALLOW.
pub fn inspect_signed_roster(
    signed: &OwnerSignedRoster,
    pinned: &IndependentlyPinnedOwner,
) -> Result<VerifiedRosterObservation, RosterError> {
    if pinned.owner_public_key == [0; 32]
        || pinned.vault_id.as_uuid().is_nil()
        || pinned.witnessed_min_epoch == 0
    {
        return Err(RosterError::MissingIndependentPin);
    }
    if signed.vault_id != pinned.vault_id {
        return Err(RosterError::WrongVault);
    }
    if signed.epoch < pinned.witnessed_min_epoch {
        return Err(RosterError::Rollback);
    }
    let bytes = signed.canonical_claim()?;
    UnparsedPublicKey::new(&signature::ED25519, pinned.owner_public_key)
        .verify(&bytes, &signed.signature)
        .map_err(|_| RosterError::InvalidSignature)?;
    if signed.epoch == pinned.witnessed_min_epoch {
        let expected_digest = pinned
            .witnessed_epoch_digest
            .ok_or(RosterError::MissingIndependentPin)?;
        if signed.claim_digest()? != expected_digest {
            return Err(RosterError::WitnessMismatch);
        }
    }
    Ok(VerifiedRosterObservation {
        vault_id: signed.vault_id,
        epoch: signed.epoch,
        devices: signed
            .devices
            .iter()
            .map(|device| ObservedDevice {
                id: device.device_id,
                device_public_key: device.device_public_key,
                endpoint_public_key: device.endpoint_public_key,
                status: device.status,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn owner(seed: u8) -> Ed25519KeyPair {
        Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap()
    }

    fn sign(mut claim: OwnerSignedRoster, signer: &Ed25519KeyPair) -> OwnerSignedRoster {
        let signature = signer.sign(&claim.canonical_claim().unwrap());
        claim.signature.copy_from_slice(signature.as_ref());
        claim
    }

    fn fixture() -> (OwnerSignedRoster, IndependentlyPinnedOwner) {
        let vault_id = VaultId::new();
        let signer = owner(11);
        let pinned = IndependentlyPinnedOwner {
            vault_id,
            owner_public_key: signer.public_key().as_ref().try_into().unwrap(),
            witnessed_min_epoch: 7,
            witnessed_epoch_digest: None,
        };
        let claim = OwnerSignedRoster {
            vault_id,
            epoch: 8,
            devices: vec![ClaimedDevice {
                device_id: DeviceId::new(),
                device_public_key: [2; 32],
                endpoint_public_key: [3; 32],
                status: RosterStatus::RecordedOnly,
            }],
            signature: [0; 64],
        };
        (sign(claim, &signer), pinned)
    }

    #[test]
    fn authentic_roster_is_recorded_only_without_any_positive_grant() {
        let (roster, pinned) = fixture();
        let observed = inspect_signed_roster(&roster, &pinned).unwrap();
        assert_eq!(observed.vault_id(), pinned.vault_id);
        assert_eq!(observed.epoch(), 8);
        assert_eq!(observed.devices().len(), 1);
        assert_eq!(observed.devices()[0].status(), RosterStatus::RecordedOnly);
        assert_eq!(
            observed.devices()[0].device_id(),
            roster.devices[0].device_id
        );
        assert_eq!(observed.devices()[0].endpoint_public_key(), [3; 32]);
        assert_ne!(observed.devices()[0].display_fingerprint(), [0; 32]);
    }

    #[test]
    fn mutating_identity_endpoint_revocation_or_epoch_breaks_signature() {
        let (roster, pinned) = fixture();
        let mut changed = roster.clone();
        changed.devices[0].device_public_key = [4; 32];
        assert_eq!(
            inspect_signed_roster(&changed, &pinned),
            Err(RosterError::InvalidSignature)
        );
        let mut changed = roster.clone();
        changed.devices[0].endpoint_public_key = [5; 32];
        assert_eq!(
            inspect_signed_roster(&changed, &pinned),
            Err(RosterError::InvalidSignature)
        );
        let mut changed = roster.clone();
        changed.devices[0].status = RosterStatus::Revoked;
        assert_eq!(
            inspect_signed_roster(&changed, &pinned),
            Err(RosterError::InvalidSignature)
        );
        let mut changed = roster;
        changed.epoch += 1;
        assert_eq!(
            inspect_signed_roster(&changed, &pinned),
            Err(RosterError::InvalidSignature)
        );
    }

    #[test]
    fn wrong_owner_vault_replay_and_missing_witness_fail_closed() {
        let (roster, pinned) = fixture();
        let mut other = pinned;
        other.owner_public_key = owner(12).public_key().as_ref().try_into().unwrap();
        assert_eq!(
            inspect_signed_roster(&roster, &other),
            Err(RosterError::InvalidSignature)
        );
        let mut other = pinned;
        other.vault_id = VaultId::new();
        assert_eq!(
            inspect_signed_roster(&roster, &other),
            Err(RosterError::WrongVault)
        );
        let mut other = pinned;
        other.witnessed_min_epoch = 9;
        assert_eq!(
            inspect_signed_roster(&roster, &other),
            Err(RosterError::Rollback)
        );
        let mut other = pinned;
        other.witnessed_min_epoch = 0;
        assert_eq!(
            inspect_signed_roster(&roster, &other),
            Err(RosterError::MissingIndependentPin)
        );
    }

    #[test]
    fn duplicates_colliding_endpoints_and_oversize_are_ambiguous() {
        let (roster, pinned) = fixture();
        let mut duplicate = roster.clone();
        duplicate.devices.push(duplicate.devices[0].clone());
        assert_eq!(
            inspect_signed_roster(&duplicate, &pinned),
            Err(RosterError::AmbiguousIdentity)
        );
        let mut collision = roster.clone();
        let mut second = collision.devices[0].clone();
        second.device_id = DeviceId::new();
        collision.devices.push(second);
        collision.devices.sort_by_key(|item| item.device_id);
        assert_eq!(
            inspect_signed_roster(&collision, &pinned),
            Err(RosterError::AmbiguousIdentity)
        );
        let mut huge = roster;
        huge.devices = (0..=MAX_ROSTER_DEVICES)
            .map(|_| ClaimedDevice {
                device_id: DeviceId::new(),
                device_public_key: [2; 32],
                endpoint_public_key: [3; 32],
                status: RosterStatus::RecordedOnly,
            })
            .collect();
        assert_eq!(
            inspect_signed_roster(&huge, &pinned),
            Err(RosterError::Oversize)
        );
    }

    #[test]
    fn same_epoch_signed_fork_is_rejected_by_independent_witness() {
        let (roster, mut pinned) = fixture();
        pinned.witnessed_min_epoch = roster.epoch;
        assert_eq!(
            inspect_signed_roster(&roster, &pinned),
            Err(RosterError::MissingIndependentPin)
        );
        pinned.witnessed_epoch_digest = Some(roster.claim_digest().unwrap());
        assert!(inspect_signed_roster(&roster, &pinned).is_ok());

        // Both snapshots are owner-signed and have the same epoch; the
        // external digest, not the signature alone, prevents a fork.
        let mut conflicting = roster.clone();
        conflicting.devices[0].status = RosterStatus::Revoked;
        let conflicting = sign(conflicting, &owner(11));
        assert_eq!(conflicting.epoch, roster.epoch);
        assert_eq!(
            inspect_signed_roster(&conflicting, &pinned),
            Err(RosterError::WitnessMismatch)
        );
    }

    #[test]
    fn freshly_signed_revocation_is_visible_but_cannot_authorize_anything() {
        let (mut roster, pinned) = fixture();
        roster.devices[0].status = RosterStatus::Revoked;
        roster.epoch = 9;
        let revoked = sign(roster, &owner(11));
        let observed = inspect_signed_roster(&revoked, &pinned).unwrap();
        assert_eq!(observed.devices()[0].status(), RosterStatus::Revoked);
        let later = IndependentlyPinnedOwner {
            witnessed_min_epoch: 10,
            ..pinned
        };
        assert_eq!(
            inspect_signed_roster(&revoked, &later),
            Err(RosterError::Rollback)
        );
    }
}
