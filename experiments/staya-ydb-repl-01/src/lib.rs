//! STAYA YDB-REPL-01: isolate the actual LesHa replication planner from Akula
//! while testing the gate it EXPECTS at the prevalidated receipt boundary.
//!
//! This is a synthetic in-memory disk and key-enrollment fixture.
//! Its simulated commit flag is NOT fsync, power-loss proof or remote-disk proof.
//! Never expose new_prevalidated to remote messages or product code.

use std::collections::BTreeMap;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use lesha_replication::{
    ProtectedSubject, ReceiptId, ReplicaSubject, VerifiedReplicaReceipt,
};
use lesha_types::{NodeId, NodeRef};
use sha2::{Digest, Sha256};

const SIGNING_DOMAIN: &[u8] = b"akula/staya/ydb-repl-01/receipt-fixture-v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionError {
    WrongSubject,
    UnknownSigner,
    WrongGeneration,
    BadSignature,
    NotCommitted,
    DigestMismatch,
}

#[derive(Clone, Debug)]
pub struct SyntheticReceipt {
    pub receipt_id: ReceiptId,
    pub node: NodeRef,
    pub subject: ProtectedSubject,
    pub storage_generation: u64,
    pub declared_committed: bool,
    signature: [u8; 64],
}

impl SyntheticReceipt {
    pub fn sign(
        key: &SigningKey,
        receipt_id: ReceiptId,
        node: NodeRef,
        subject: ProtectedSubject,
        storage_generation: u64,
        declared_committed: bool,
    ) -> Self {
        let mut value = Self {
            receipt_id,
            node,
            subject,
            storage_generation,
            declared_committed,
            signature: [0; 64],
        };
        value.signature = key.sign(&value.signed_bytes()).to_bytes();
        value
    }

    fn signed_bytes(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(180);
        data.extend_from_slice(SIGNING_DOMAIN);
        data.extend_from_slice(&self.receipt_id.0);
        data.extend_from_slice(&self.node.cluster_id.0);
        data.extend_from_slice(&self.node.node_id.0);
        data.extend_from_slice(&self.node.generation.0.to_be_bytes());
        data.extend_from_slice(&self.subject.cluster_id.0);
        match self.subject.subject {
            ReplicaSubject::Chunk(id) => {
                data.push(1);
                data.extend_from_slice(&id.0);
            }
            ReplicaSubject::VersionBundle(id) => {
                data.push(2);
                data.extend_from_slice(&id.0);
            }
        }
        data.extend_from_slice(&self.subject.subject_hash);
        data.extend_from_slice(&self.storage_generation.to_be_bytes());
        data.push(u8::from(self.declared_committed));
        data
    }
}

#[derive(Clone, Debug)]
struct SyntheticFile {
    storage_generation: u64,
    bytes: Vec<u8>,
    committed: bool,
}

/// One process pretends to be multiple machines. A real sender cannot prove
/// remote durable storage by inspecting an in-memory map like this.
#[derive(Default)]
pub struct SyntheticDiskCluster {
    files: BTreeMap<NodeId, SyntheticFile>,
}

impl SyntheticDiskCluster {
    pub fn stage(&mut self, node: NodeRef, storage_generation: u64, bytes: Vec<u8>) {
        self.files.insert(node.node_id, SyntheticFile {
            storage_generation, bytes, committed: false,
        });
    }

    /// A mock state transition, intentionally NOT a durable disk operation.
    pub fn simulated_commit(&mut self, node: NodeRef) {
        self.files.get_mut(&node.node_id).expect("must be staged").committed = true;
    }

    pub fn corrupt(&mut self, node: NodeRef) {
        let f = self.files.get_mut(&node.node_id).expect("must be staged");
        if f.bytes.is_empty() { f.bytes.push(0xEE); }
        else { f.bytes[0] ^= 0x80; }
    }

    pub fn committed_bytes(&self, node: NodeRef) -> Option<&[u8]> {
        self.files.get(&node.node_id).and_then(|f| {
            if f.committed { Some(f.bytes.as_slice()) } else { None }
        })
    }
}

#[derive(Clone, Copy)]
struct RegisteredNode {
    node: NodeRef,
    public_key: VerifyingKey,
    storage_generation: u64,
}

/// Admission boundary: cryptographic signature + enrolled generation + exact
/// target + synthetic disk readback. Not a production remote receipt verifier.
#[derive(Default)]
pub struct SyntheticReceiptGate {
    enrolled: BTreeMap<NodeId, RegisteredNode>,
}

impl SyntheticReceiptGate {
    pub fn enroll(&mut self, node: NodeRef, public_key: VerifyingKey, storage_generation: u64) {
        self.enrolled.insert(node.node_id, RegisteredNode {
            node, public_key, storage_generation,
        });
    }

    pub fn admit(
        &self,
        disks: &SyntheticDiskCluster,
        target: &ProtectedSubject,
        claimed: &SyntheticReceipt,
    ) -> Result<VerifiedReplicaReceipt, AdmissionError> {
        if claimed.subject != *target || claimed.node.cluster_id != target.cluster_id {
            return Err(AdmissionError::WrongSubject);
        }
        let trusted = self.enrolled.get(&claimed.node.node_id)
            .ok_or(AdmissionError::UnknownSigner)?;
        if trusted.node != claimed.node ||
            trusted.storage_generation != claimed.storage_generation
        {
            return Err(AdmissionError::WrongGeneration);
        }
        trusted.public_key.verify_strict(
            &claimed.signed_bytes(),
            &Signature::from_bytes(&claimed.signature),
        ).map_err(|_| AdmissionError::BadSignature)?;

        if !claimed.declared_committed {
            return Err(AdmissionError::NotCommitted);
        }
        let local = disks.files.get(&claimed.node.node_id)
            .ok_or(AdmissionError::NotCommitted)?;
        if !local.committed {
            return Err(AdmissionError::NotCommitted);
        }
        if local.storage_generation != claimed.storage_generation {
            return Err(AdmissionError::WrongGeneration);
        }
        let actual = Sha256::digest(&local.bytes);
        if actual[..] != target.subject_hash[..] {
            return Err(AdmissionError::DigestMismatch);
        }
        match target.subject {
            ReplicaSubject::Chunk(id) if id.0[..] != actual[..] => {
                return Err(AdmissionError::DigestMismatch);
            }
            _ => {}
        }

        // This is precisely the trust boundary in LesHa's pure algorithm:
        // NEVER create a VerifiedReplicaReceipt directly from gossip bytes.
        Ok(VerifiedReplicaReceipt::new_prevalidated(
            claimed.receipt_id,
            target.cluster_id,
            claimed.node,
            target.subject,
            target.subject_hash,
            claimed.storage_generation,
        ))
    }
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut result = [0u8; 32];
    result.copy_from_slice(&Sha256::digest(bytes)[..]);
    result
}

#[cfg(test)]
mod tests;
