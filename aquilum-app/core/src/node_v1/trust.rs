//! Root-provisioned owner policy. Node never accepts keys, epochs or digests
//! from peer requests, CLI arguments, tickets, or editable service data.

use super::NodeV1Error;
use super::put::OwnerPutGrant;
use crate::node_replica_auth_lab::IndependentReadTrust;
use crate::sync::device_roster_lab::IndependentlyPinnedOwner;
use crate::sync::VaultId;
use rustix::fs::{open, openat, Mode, OFlags};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

mod admit;
#[cfg(test)]
mod fixtures;
#[cfg(test)]
pub(super) use fixtures::{admitted_fixture, policy_fixture};

const POLICY_NAME: &str = "owner-policy.json";
const POLICY_PARENT: &str = "/etc/akula-node";
const MAX_POLICY_BYTES: u64 = 2048;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    version: u32,
    node_public: [u8; 32],
    iroh_endpoint_public: [u8; 32],
    owner_public: [u8; 32],
    vault_id: VaultId,
    grant_epoch: u64,
    roster_digest: [u8; 32],
    grant_digest: [u8; 32],
    put_grant_digest: [u8; 32],
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

/// A signed write approval bound to an independently pinned owner policy.
/// This type cannot be constructed by the network layer.
pub struct AdmittedPut {
    grant: OwnerPutGrant,
    digest: [u8; 32],
    policy_digest: [u8; 32],
}
impl AdmittedPut {
    pub(super) fn scope(&self) -> &OwnerPutGrant { &self.grant }
    pub(super) fn digest(&self) -> [u8; 32] { self.digest }
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
    #[cfg(all(feature = "node-v1-transport-tests", debug_assertions))]
    pub fn open_integration_fixture(
        parent: &Path, node_public: [u8; 32],
    ) -> Result<Self, NodeV1Error> {
        Self::load_from(parent, rustix::process::geteuid().as_raw(), node_public)
    }

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
        if policy.version != 3 || policy.node_public != node_public
            || policy.iroh_endpoint_public == [0; 32]
            || node_public == [0; 32] || policy.owner_public == [0; 32]
            || policy.vault_id.as_uuid().is_nil() || policy.grant_epoch == 0
            || policy.roster_digest == [0; 32] || policy.grant_digest == [0; 32]
            || policy.put_grant_digest == [0; 32]
        {
            return Err(NodeV1Error::Hold("owner policy does not bind this Node"));
        }
        let digest = *blake3::hash(&bytes).as_bytes();
        Ok(Self { parent: parent.to_path_buf(), owner_uid, policy, digest })
    }

    /// Every remote access checks that a root-only policy update or revoke
    /// has not invalidated the in-memory trusted snapshot.
    pub(super) fn policy_digest(&self) -> [u8; 32] { self.digest }

    pub(super) fn require_iroh_endpoint(&self, presented: [u8; 32])
        -> Result<(), NodeV1Error>
    {
        self.require_current()?;
        if presented != self.policy.iroh_endpoint_public {
            return Err(NodeV1Error::Hold("Iroh EndpointId differs from root policy"));
        }
        Ok(())
    }

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


}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod put_tests;
