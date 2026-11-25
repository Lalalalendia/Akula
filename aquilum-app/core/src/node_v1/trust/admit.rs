//! Read and write grants share the same signed roster, but remain separate
//! operation authorities with distinct owner-signed claims and digests.
use super::*;
use super::super::put::{put_claim, signed_put_digest};
use crate::node_replica_auth_lab::{OwnerReadGrant, owner_claim, signed_grant_digest};
use crate::sync::device_roster_lab::{OwnerSignedRoster, RosterStatus, inspect_signed_roster};
use ring::signature::{UnparsedPublicKey, ED25519};

enum DeviceGrant<'a> {
    Read(&'a OwnerReadGrant),
    Put(&'a OwnerPutGrant),
}

impl RootTrust {
    fn require_grant_device(
        &self, roster: &OwnerSignedRoster, peer: [u8; 32], grant: DeviceGrant<'_>,
    ) -> Result<(), NodeV1Error> {
        self.require_current()?;
        let (device_id, device_key, node, vault, epoch, errors) = match grant {
            DeviceGrant::Read(g) => (g.device_id, g.device_public_key,
                g.node_identity, g.vault_id, g.grant_epoch,
                ["incorrect read scope", "invalid signed roster",
                    "device is not enrolled", "revoked or ambiguous device"]),
            DeviceGrant::Put(g) => (g.device_id, g.device_public_key,
                g.node_identity, g.vault_id, g.grant_epoch,
                ["incorrect write scope", "invalid signed device roster",
                    "write device is not enrolled", "revoked write device"]),
        };
        let [bad_scope, bad_roster, missing_device, revoked_device] = errors;
        if peer == [0; 32] || device_key != peer
            || node != self.policy.node_public
            || vault != self.policy.vault_id
            || epoch != self.policy.grant_epoch
            || roster.epoch != self.policy.grant_epoch
        {
            return Err(NodeV1Error::Hold(bad_scope));
        }
        let observed = inspect_signed_roster(roster, &self.witnessed_roster()?)
            .map_err(|_| NodeV1Error::Hold(bad_roster))?;
        let mut devices = observed.devices().iter()
            .filter(|device| device.endpoint_public_key() == peer);
        let device = devices.next().ok_or(NodeV1Error::Hold(missing_device))?;
        if devices.next().is_some()
            || device.status() != RosterStatus::RecordedOnly
            || device.device_public_key() != peer
            || device.device_id() != device_id
        {
            return Err(NodeV1Error::Hold(revoked_device));
        }
        Ok(())
    }

    /// A signed grant still requires the exact current root policy and active signed roster.
    pub fn admit_put(
        &self, roster: &OwnerSignedRoster, grant: &OwnerPutGrant,
        peer: [u8; 32],
    ) -> Result<AdmittedPut, NodeV1Error> {
        self.require_grant_device(roster, peer, DeviceGrant::Put(grant))?;
        let claim = put_claim(grant)
            .map_err(|_| NodeV1Error::Hold("invalid write claim"))?;
        UnparsedPublicKey::new(&ED25519, &self.policy.owner_public)
            .verify(&claim, &grant.owner_signature)
            .map_err(|_| NodeV1Error::Hold("invalid owner write signature"))?;
        let digest = signed_put_digest(grant)
            .map_err(|_| NodeV1Error::Hold("invalid signed write digest"))?;
        if digest != self.policy.put_grant_digest {
            return Err(NodeV1Error::Hold("owner write grant is no longer current"));
        }
        Ok(AdmittedPut {
            grant: grant.clone(), digest, policy_digest: self.digest,
        })
    }

    pub fn admit_read(
        &self, roster: &OwnerSignedRoster, grant: &OwnerReadGrant, peer: [u8; 32],
    ) -> Result<AdmittedRead, NodeV1Error> {
        self.require_grant_device(roster, peer, DeviceGrant::Read(grant))?;
        let claim = owner_claim(grant)
            .map_err(|_| NodeV1Error::Hold("invalid signed grant claim"))?;
        UnparsedPublicKey::new(&ED25519, &self.policy.owner_public)
            .verify(&claim, &grant.owner_signature)
            .map_err(|_| NodeV1Error::Hold("invalid owner signature"))?;
        let grant_digest = signed_grant_digest(grant)
            .map_err(|_| NodeV1Error::Hold("invalid grant digest"))?;
        if grant_digest != self.policy.grant_digest {
            return Err(NodeV1Error::Hold("grant is no longer current"));
        }
        Ok(AdmittedRead { peer, grant_digest, policy_digest: self.digest })
    }
}
