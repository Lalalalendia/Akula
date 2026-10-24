//! Signed, exact-revision local commit receipt. This is NOT a remote
//! replication proof: the caller must check NodeId and hold a current owner
//! policy independently. Filesystem rollback by hostile root is out of scope.

use super::{NodeV1Error, NodeIdentity};
use super::store::StoredRevision;
use crate::sync::{DocumentId, RevisionId, VaultId};
use ring::signature::{UnparsedPublicKey, ED25519};

const DOMAIN: &[u8] = b"Akula/NodeV1/local-durable-commit/v1\0";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodePutReceipt {
    pub node_public: [u8; 32],
    pub vault_id: VaultId,
    pub document_id: DocumentId,
    pub revision_id: RevisionId,
    pub raw_blake3: String,
    pub byte_length: u64,
    pub owner_epoch: u64,
    pub put_grant_digest: [u8; 32],
    pub signature: [u8; 64],
}

impl NodePutReceipt {
    pub fn claim(&self) -> Result<Vec<u8>, NodeV1Error> {
        if self.node_public == [0; 32]
            || self.put_grant_digest == [0; 32] || self.owner_epoch == 0
            || self.raw_blake3.len() != 64
            || !self.raw_blake3.bytes().all(|b| {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            })
        {
            return Err(NodeV1Error::Hold("invalid Node receipt claim"));
        }
        let mut out = Vec::with_capacity(DOMAIN.len() + 192);
        out.extend_from_slice(DOMAIN);
        out.extend_from_slice(&self.node_public);
        for id in [
            self.vault_id.as_uuid(), self.document_id.as_uuid(),
            self.revision_id.as_uuid(),
        ] {
            out.extend_from_slice(id.as_bytes());
        }
        out.extend_from_slice(self.raw_blake3.as_bytes());
        out.extend_from_slice(&self.byte_length.to_be_bytes());
        out.extend_from_slice(&self.owner_epoch.to_be_bytes());
        out.extend_from_slice(&self.put_grant_digest);
        Ok(out)
    }

    pub fn verify(&self, pinned_node: [u8; 32])
        -> Result<(), NodeV1Error>
    {
        if self.node_public != pinned_node {
            return Err(NodeV1Error::Hold("Node receipt has wrong NodeId"));
        }
        UnparsedPublicKey::new(&ED25519, pinned_node)
            .verify(&self.claim()?, &self.signature)
            .map_err(|_| NodeV1Error::Hold("invalid Node signed receipt"))
    }
}

pub(super) fn sign_local_receipt(
    identity: &NodeIdentity, stored: StoredRevision,
    epoch: u64, grant_digest: [u8; 32],
) -> Result<NodePutReceipt, NodeV1Error> {
    let mut result = NodePutReceipt {
        node_public: identity.public_key(),
        vault_id: stored.vault_id,
        document_id: stored.document_id,
        revision_id: stored.revision_id,
        raw_blake3: stored.raw_blake3,
        byte_length: stored.byte_length,
        owner_epoch: epoch,
        put_grant_digest: grant_digest,
        signature: [0; 64],
    };
    let signature = identity.sign_receipt_claim(&result.claim()?);
    result.signature = signature;
    result.verify(identity.public_key())?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn signed_receipt_binds_node_revision_hash_and_grant() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let node = NodeIdentity::open_or_create(&state).unwrap();
        let scope = StoredRevision {
            vault_id: VaultId::new(),
            document_id: DocumentId::new(),
            revision_id: RevisionId::new(),
            raw_blake3: blake3::hash(b"one").to_hex().to_string(),
            byte_length: 3,
        };
        let receipt = sign_local_receipt(&node, scope, 12, [7; 32]).unwrap();
        assert!(receipt.verify(node.public_key()).is_ok());
        assert!(receipt.verify([8; 32]).is_err());
        let mut corrupted = receipt.clone();
        corrupted.byte_length = 7;
        assert!(corrupted.verify(node.public_key()).is_err());
        corrupted = receipt;
        corrupted.put_grant_digest[0] ^= 1;
        assert!(corrupted.verify(node.public_key()).is_err());
    }
}
