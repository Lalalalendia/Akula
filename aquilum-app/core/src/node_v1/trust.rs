//! Root-provisioned owner policy. Node never accepts keys, epochs or digests
//! from peer requests, CLI arguments, tickets, or editable service data.

use super::NodeV1Error;
use crate::node_replica_auth_lab::{
    IndependentReadTrust, OwnerReadGrant, owner_claim, signed_grant_digest,
};
use crate::sync::device_roster_lab::{
    IndependentlyPinnedOwner, OwnerSignedRoster, RosterStatus, inspect_signed_roster,
};
use crate::sync::VaultId;
use ring::signature::{UnparsedPublicKey, ED25519};
use rustix::fs::{open, openat, Mode, OFlags};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

const POLICY_NAME: &str = "owner-policy.json";
const POLICY_PARENT: &str = "/etc/akula-node";
const MAX_POLICY_BYTES: u64 = 2048;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    version: u32,
    node_public: [u8; 32],
    owner_public: [u8; 32],
    vault_id: VaultId,
    grant_epoch: u64,
    roster_digest: [u8; 32],
    grant_digest: [u8; 32],
}

/// This token can be constructed only after comparing an owner-signed roster
/// and grant to the independently protected root policy.
pub struct AdmittedRead {
    peer: [u8; 32],
    grant_digest: [u8; 32],
    policy_digest: [u8; 32],
}
impl AdmittedRead {
    pub(super) fn peer(&self) -> [u8; 32] { self.peer }
    pub(super) fn grant_digest(&self) -> [u8; 32] { self.grant_digest }
    pub(super) fn policy_digest(&self) -> [u8; 32] { self.policy_digest }
}

#[derive(Clone)]
pub struct RootTrust {
    parent: PathBuf,
    owner_uid: u32,
    policy: PolicyFile,
    digest: [u8; 32],
}

impl RootTrust {
    pub fn open_provisioned(node_public: [u8; 32]) -> Result<Self, NodeV1Error> {
        // A malicious same-UID service cannot replace a root-owned /etc tree.
        for path in ["/", "/etc"] {
            let meta = fs::symlink_metadata(path)?;
            if !meta.is_dir() || meta.uid() != 0
                || meta.permissions().mode() & 0o022 != 0
            {
                return Err(NodeV1Error::Hold("untrusted root policy ancestors"));
            }
        }
        Self::load_from(Path::new(POLICY_PARENT), 0, node_public)
    }

    fn load_from(parent: &Path, owner_uid: u32, node_public: [u8; 32])
        -> Result<Self, NodeV1Error>
    {
        let meta = fs::symlink_metadata(parent)?;
        if !meta.is_dir() || meta.uid() != owner_uid
            || meta.permissions().mode() & 0o022 != 0
        {
            return Err(NodeV1Error::Hold("untrusted owner-policy directory"));
        }
        let dir = File::from(open(
            parent, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW
                | OFlags::CLOEXEC, Mode::empty(),
        ).map_err(std::io::Error::from)?);
        let physical = dir.metadata()?;
        if physical.dev() != meta.dev() || physical.ino() != meta.ino() {
            return Err(NodeV1Error::Hold("owner-policy directory replaced"));
        }
        let mut file = File::from(openat(
            &dir, POLICY_NAME, OFlags::RDONLY | OFlags::NOFOLLOW
                | OFlags::CLOEXEC, Mode::empty(),
        ).map_err(std::io::Error::from)?);
        let info = file.metadata()?;
        if !info.is_file() || info.uid() != owner_uid || info.nlink() != 1
            || info.permissions().mode() & 0o022 != 0
            || info.len() == 0 || info.len() > MAX_POLICY_BYTES
        {
            return Err(NodeV1Error::Hold("unsafe owner-policy file"));
        }
        let mut bytes = Vec::with_capacity(info.len() as usize);
        file.take(MAX_POLICY_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_POLICY_BYTES {
            return Err(NodeV1Error::Hold("oversize owner policy"));
        }
        let policy: PolicyFile = serde_json::from_slice(&bytes)
            .map_err(|_| NodeV1Error::Hold("invalid owner policy"))?;
        if policy.version != 1 || policy.node_public != node_public
            || node_public == [0; 32] || policy.owner_public == [0; 32]
            || policy.vault_id.as_uuid().is_nil() || policy.grant_epoch == 0
            || policy.roster_digest == [0; 32] || policy.grant_digest == [0; 32]
        {
            return Err(NodeV1Error::Hold("owner policy does not bind this Node"));
        }
        let digest = *blake3::hash(&bytes).as_bytes();
        Ok(Self { parent: parent.to_path_buf(), owner_uid, policy, digest })
    }

    /// Every remote access checks that a root-only policy update or revoke
    /// has not invalidated the in-memory trusted snapshot.
    pub(super) fn policy_digest(&self) -> [u8; 32] { self.digest }

    pub fn require_current(&self) -> Result<(), NodeV1Error> {
        let updated = Self::load_from(
            &self.parent, self.owner_uid, self.policy.node_public,
        )?;
        if updated.digest != self.digest {
            return Err(NodeV1Error::Hold("owner policy changed: reload and review"));
        }
        Ok(())
    }

    pub fn witnessed_roster(&self) -> Result<IndependentlyPinnedOwner, NodeV1Error> {
        self.require_current()?;
        Ok(IndependentlyPinnedOwner {
            vault_id: self.policy.vault_id,
            owner_public_key: self.policy.owner_public,
            witnessed_min_epoch: self.policy.grant_epoch,
            witnessed_epoch_digest: Some(self.policy.roster_digest),
        })
    }

    pub fn witnessed_read(&self, challenge: [u8; 32])
        -> Result<IndependentReadTrust, NodeV1Error>
    {
        self.require_current()?;
        if challenge == [0; 32] {
            return Err(NodeV1Error::Hold("empty session challenge"));
        }
        Ok(IndependentReadTrust {
            trusted_vault_id: self.policy.vault_id,
            trusted_owner_public_key: self.policy.owner_public,
            trusted_node_identity: self.policy.node_public,
            witnessed_current_epoch: self.policy.grant_epoch,
            witnessed_exact_grant_digest: self.policy.grant_digest,
            current_session_challenge: challenge,
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

#[cfg(test)]
pub(super) fn policy_fixture(parent: &Path, node_public: [u8; 32]) -> RootTrust {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir(parent).unwrap();
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).unwrap();
    let policy = PolicyFile {
        version: 1,
        node_public,
        owner_public: [11; 32],
        vault_id: VaultId::new(),
        grant_epoch: 7,
        roster_digest: [21; 32],
        grant_digest: [31; 32],
    };
    let file = parent.join(POLICY_NAME);
    fs::write(&file, serde_json::to_vec(&policy).unwrap()).unwrap();
    fs::set_permissions(file, fs::Permissions::from_mode(0o600)).unwrap();
    RootTrust::load_from(parent, rustix::process::geteuid().as_raw(), node_public).unwrap()
}

#[cfg(test)]
pub(super) fn admitted_fixture(policy: &RootTrust) -> AdmittedRead {
    AdmittedRead {
        peer: [7; 32],
        grant_digest: policy.policy.grant_digest,
        policy_digest: policy.digest,
    }
}

#[cfg(test)]
mod tests;
