//! Read and write grants share the same signed roster, but remain separate
//! operation authorities with distinct owner-signed claims and digests.
use super::*;
use super::super::put::{put_claim, signed_put_digest};
use crate::node_replica_auth_lab::{OwnerReadGrant, owner_claim, signed_grant_digest};
use crate::sync::device_roster_lab::{OwnerSignedRoster, RosterStatus, inspect_signed_roster};
use ring::signature::{UnparsedPublicKey, ED25519};

impl RootTrust {
    /// An already signed grant is not sufficient: the exact *current*
    /// root-owned policy and owner-signed active device roster must agree.
    pub fn admit_put(
        &self, roster: &OwnerSignedRoster, grant: &OwnerPutGrant,
        peer: [u8; 32],
    ) -> Result<AdmittedPut, NodeV1Error> {
        self.require_current()?;
        if peer == [0; 32] || grant.device_public_key != peer
            || grant.node_identity != self.policy.node_public
            || grant.vault_id != self.policy.vault_id
            || grant.grant_epoch != self.policy.grant_epoch
            || roster.epoch != self.policy.grant_epoch
        {
            return Err(NodeV1Error::Hold("incorrect write scope"));
        }
        let observed = inspect_signed_roster(roster, &self.witnessed_roster()?)
            .map_err(|_| NodeV1Error::Hold("invalid signed device roster"))?;
        let mut devices = observed.devices().iter()
            .filter(|device| device.endpoint_public_key() == peer);
        let device = devices.next()
            .ok_or(NodeV1Error::Hold("write device is not enrolled"))?;
        if devices.next().is_some()
            || device.status() != RosterStatus::RecordedOnly
            || device.device_public_key() != peer
            || device.device_id() != grant.device_id
        {
            return Err(NodeV1Error::Hold("revoked write device"));
        }
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
        self.require_current()?;
        if peer == [0; 32] || grant.device_public_key != peer
            || grant.node_identity != self.policy.node_public
            || grant.vault_id != self.policy.vault_id
            || grant.grant_epoch != self.policy.grant_epoch
            || roster.epoch != self.policy.grant_epoch
        {
            return Err(NodeV1Error::Hold("incorrect read scope"));
        }
        let observed = inspect_signed_roster(roster, &self.witnessed_roster()?)
            .map_err(|_| NodeV1Error::Hold("invalid signed roster"))?;
        let mut devices = observed.devices().iter()
            .filter(|device| device.endpoint_public_key() == peer);
        let device = devices.next()
            .ok_or(NodeV1Error::Hold("device is not enrolled"))?;
        if devices.next().is_some()
            || device.status() != RosterStatus::RecordedOnly
            || device.device_public_key() != peer
            || device.device_id() != grant.device_id
        {
            return Err(NodeV1Error::Hold("revoked or ambiguous device"));
        }
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
