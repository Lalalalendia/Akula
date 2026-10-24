//! Bounded, versioned NodeV1 control messages. This encoding is NOT a grant:
//! exact owner signatures, root-witnessed digests, and live Iroh remote_id
//! are verified separately before any bytes are read from NodeStore.
use super::NodeWireDenied;
use aquilum_core::node_replica_auth_lab::OwnerReadGrant;
use aquilum_core::node_v1::{NodePutReceipt, OwnerPutGrant};
use aquilum_core::sync::device_roster_lab::{ClaimedDevice, OwnerSignedRoster, RosterStatus};
use aquilum_core::sync::{DeviceId, DocumentId, RevisionId, VaultId};
use serde::{Deserialize, Serialize};

pub(super) const MAX_CONTROL: usize = 32 * 1024;
pub(super) const MAX_ACK: usize = 4096;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireDevice {
    device_id: DeviceId,
    device_public_key: [u8; 32],
    endpoint_public_key: [u8; 32],
    status: u8,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireRoster {
    vault_id: VaultId,
    epoch: u64,
    devices: Vec<WireDevice>,
    signature: Vec<u8>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireGrant {
    vault_id: VaultId,
    document_id: DocumentId,
    revision_id: RevisionId,
    filename: String,
    expected_blake3: String,
    node_identity: [u8; 32],
    device_id: DeviceId,
    device_public_key: [u8; 32],
    grant_epoch: u64,
    owner_signature: Vec<u8>,
}
#[derive(Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operation {
    Put,
    Get,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OpenRequest {
    pub(super) version: u8,
    pub(super) operation: Operation,
    pub(super) client_nonce: [u8; 32],
    pub(super) roster: WireRoster,
    pub(super) grant: WireGrant,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChallengeReply {
    pub(super) version: u8,
    pub(super) node_public: [u8; 32],
    pub(super) health_signature: Vec<u8>,
    pub(super) challenge: [u8; 32],
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireReceipt {
    node_public: [u8; 32],
    vault_id: VaultId,
    document_id: DocumentId,
    revision_id: RevisionId,
    raw_blake3: String,
    byte_length: u64,
    owner_epoch: u64,
    put_grant_digest: [u8; 32],
    signature: Vec<u8>,
}
fn signature(bytes: Vec<u8>) -> Result<[u8; 64], NodeWireDenied> {
    bytes.try_into().map_err(|_| NodeWireDenied)
}

impl From<&OwnerSignedRoster> for WireRoster {
    fn from(value: &OwnerSignedRoster) -> Self {
        Self {
            vault_id: value.vault_id,
            epoch: value.epoch,
            devices: value
                .devices
                .iter()
                .map(|d| WireDevice {
                    device_id: d.device_id,
                    device_public_key: d.device_public_key,
                    endpoint_public_key: d.endpoint_public_key,
                    status: match d.status {
                        RosterStatus::RecordedOnly => 1,
                        RosterStatus::Revoked => 2,
                    },
                })
                .collect(),
            signature: value.signature.to_vec(),
        }
    }
}
impl TryFrom<WireRoster> for OwnerSignedRoster {
    type Error = NodeWireDenied;
    fn try_from(wire: WireRoster) -> Result<Self, Self::Error> {
        if wire.devices.len() > 32 {
            return Err(NodeWireDenied);
        }
        let devices = wire
            .devices
            .into_iter()
            .map(|d| {
                Ok(ClaimedDevice {
                    device_id: d.device_id,
                    device_public_key: d.device_public_key,
                    endpoint_public_key: d.endpoint_public_key,
                    status: match d.status {
                        1 => RosterStatus::RecordedOnly,
                        2 => RosterStatus::Revoked,
                        _ => return Err(NodeWireDenied),
                    },
                })
            })
            .collect::<Result<Vec<_>, NodeWireDenied>>()?;
        Ok(Self {
            vault_id: wire.vault_id,
            epoch: wire.epoch,
            devices,
            signature: signature(wire.signature)?,
        })
    }
}
impl From<&OwnerPutGrant> for WireGrant {
    fn from(g: &OwnerPutGrant) -> Self {
        Self {
            vault_id: g.vault_id,
            document_id: g.document_id,
            revision_id: g.revision_id,
            filename: g.filename.clone(),
            expected_blake3: g.expected_blake3.clone(),
            node_identity: g.node_identity,
            device_id: g.device_id,
            device_public_key: g.device_public_key,
            grant_epoch: g.grant_epoch,
            owner_signature: g.owner_signature.to_vec(),
        }
    }
}
impl From<&OwnerReadGrant> for WireGrant {
    fn from(g: &OwnerReadGrant) -> Self {
        Self {
            vault_id: g.vault_id,
            document_id: g.document_id,
            revision_id: g.revision_id,
            filename: g.filename.clone(),
            expected_blake3: g.expected_blake3.clone(),
            node_identity: g.node_identity,
            device_id: g.device_id,
            device_public_key: g.device_public_key,
            grant_epoch: g.grant_epoch,
            owner_signature: g.owner_signature.to_vec(),
        }
    }
}
impl WireGrant {
    pub(super) fn put(self) -> Result<OwnerPutGrant, NodeWireDenied> {
        Ok(OwnerPutGrant {
            vault_id: self.vault_id,
            document_id: self.document_id,
            revision_id: self.revision_id,
            filename: self.filename,
            expected_blake3: self.expected_blake3,
            node_identity: self.node_identity,
            device_id: self.device_id,
            device_public_key: self.device_public_key,
            grant_epoch: self.grant_epoch,
            owner_signature: signature(self.owner_signature)?,
        })
    }
    pub(super) fn read(self) -> Result<OwnerReadGrant, NodeWireDenied> {
        Ok(OwnerReadGrant {
            vault_id: self.vault_id,
            document_id: self.document_id,
            revision_id: self.revision_id,
            filename: self.filename,
            expected_blake3: self.expected_blake3,
            node_identity: self.node_identity,
            device_id: self.device_id,
            device_public_key: self.device_public_key,
            grant_epoch: self.grant_epoch,
            owner_signature: signature(self.owner_signature)?,
        })
    }
}
impl From<&NodePutReceipt> for WireReceipt {
    fn from(v: &NodePutReceipt) -> Self {
        Self {
            node_public: v.node_public,
            vault_id: v.vault_id,
            document_id: v.document_id,
            revision_id: v.revision_id,
            raw_blake3: v.raw_blake3.clone(),
            byte_length: v.byte_length,
            owner_epoch: v.owner_epoch,
            put_grant_digest: v.put_grant_digest,
            signature: v.signature.to_vec(),
        }
    }
}
impl TryFrom<WireReceipt> for NodePutReceipt {
    type Error = NodeWireDenied;
    fn try_from(v: WireReceipt) -> Result<Self, Self::Error> {
        Ok(Self {
            node_public: v.node_public,
            vault_id: v.vault_id,
            document_id: v.document_id,
            revision_id: v.revision_id,
            raw_blake3: v.raw_blake3,
            byte_length: v.byte_length,
            owner_epoch: v.owner_epoch,
            put_grant_digest: v.put_grant_digest,
            signature: signature(v.signature)?,
        })
    }
}
