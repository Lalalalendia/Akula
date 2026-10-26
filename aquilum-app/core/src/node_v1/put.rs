//! Exact, owner-signed Node replica WRITE intent. Read grants are never
//! write authority. The signed claim admits ONE immutable Markdown revision.
//! Transport and installer code must supply an authenticated Iroh peer.

use crate::sync::{DeviceId, DocumentId, RevisionId, SyncRelativePath, VaultId};
use ring::signature::{self, UnparsedPublicKey};

const PUT_DOMAIN: &[u8] = b"Akula/NodeV1/owner-put/v1\0";
const PROOF_DOMAIN: &[u8] = b"Akula/NodeV1/device-put-proof/v1\0";
const DIGEST_DOMAIN: &str = "Akula/NodeV1/owner-put-digest/v1";
pub const MAX_PUT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerPutGrant {
    pub vault_id: VaultId,
    pub document_id: DocumentId,
    pub revision_id: RevisionId,
    pub filename: String,
    pub expected_blake3: String,
    pub node_identity: [u8; 32],
    pub device_id: DeviceId,
    pub device_public_key: [u8; 32],
    pub grant_epoch: u64,
    pub owner_signature: [u8; 64],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PutDenied;

fn checked_path(grant: &OwnerPutGrant) -> Result<SyncRelativePath, PutDenied> {
    let path = SyncRelativePath::parse(&grant.filename).map_err(|_| PutDenied)?;
    if grant.filename.is_empty() || grant.filename.len() > 255
        || path.as_str() != grant.filename || path.as_str().contains('/')
        || path.as_str().contains('\\') || !path.as_str().ends_with(".md")
    {
        return Err(PutDenied);
    }
    Ok(path)
}

pub fn put_claim(grant: &OwnerPutGrant) -> Result<Vec<u8>, PutDenied> {
    let _ = checked_path(grant)?;
    let hash = grant.expected_blake3.as_bytes();
    if grant.vault_id.as_uuid().is_nil()
        || grant.document_id.as_uuid().is_nil()
        || grant.revision_id.as_uuid().is_nil()
        || grant.device_id.as_uuid().is_nil()
        || grant.node_identity == [0; 32]
        || grant.device_public_key == [0; 32]
        || grant.grant_epoch == 0
        || hash.len() != 64
        || !hash.iter().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
    {
        return Err(PutDenied);
    }
    let name = grant.filename.as_bytes();
    let mut out = Vec::with_capacity(PUT_DOMAIN.len() + 256 + name.len());
    out.extend_from_slice(PUT_DOMAIN);
    for id in [
        grant.vault_id.as_uuid(), grant.document_id.as_uuid(),
        grant.revision_id.as_uuid(), grant.device_id.as_uuid(),
    ] {
        out.extend_from_slice(id.as_bytes());
    }
    out.extend_from_slice(&grant.node_identity);
    out.extend_from_slice(&grant.device_public_key);
    out.extend_from_slice(&grant.grant_epoch.to_be_bytes());
    out.extend_from_slice(&(name.len() as u16).to_be_bytes());
    out.extend_from_slice(name);
    out.extend_from_slice(hash);
    Ok(out)
}

pub fn signed_put_digest(grant: &OwnerPutGrant) -> Result<[u8; 32], PutDenied> {
    let mut hash = blake3::Hasher::new_derive_key(DIGEST_DOMAIN);
    hash.update(&put_claim(grant)?);
    hash.update(&grant.owner_signature);
    Ok(*hash.finalize().as_bytes())
}

pub fn put_proof_claim(
    grant: &OwnerPutGrant, challenge: [u8; 32],
) -> Result<Vec<u8>, PutDenied> {
    if challenge == [0; 32] { return Err(PutDenied); }
    let mut bytes = Vec::with_capacity(PROOF_DOMAIN.len() + 96);
    bytes.extend_from_slice(PROOF_DOMAIN);
    bytes.extend_from_slice(&grant.node_identity);
    bytes.extend_from_slice(&signed_put_digest(grant)?);
    bytes.extend_from_slice(&challenge);
    Ok(bytes)
}

/// Device authorization is separate from the owner-signed immutable scope.
pub fn verify_put_proof(
    grant: &OwnerPutGrant, challenge: [u8; 32], proof: &[u8; 64],
) -> Result<(), PutDenied> {
    let claim = put_proof_claim(grant, challenge)?;
    UnparsedPublicKey::new(&signature::ED25519, &grant.device_public_key)
        .verify(&claim, proof)
        .map_err(|_| PutDenied)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_paths_hashes_and_ambiguous_challenges() {
        let mut grant = OwnerPutGrant {
            vault_id: VaultId::new(), document_id: DocumentId::new(),
            revision_id: RevisionId::new(), filename: "one.md".into(),
            expected_blake3: "f".repeat(64), node_identity: [3;32],
            device_id: DeviceId::new(), device_public_key: [4;32],
            grant_epoch: 1, owner_signature: [5;64],
        };
        assert!(put_claim(&grant).is_ok());
        assert!(put_proof_claim(&grant, [0;32]).is_err());
        for bad in ["../one.md", "a/b.md", "a\\b.md", "one.txt", "."] {
            grant.filename = bad.into();
            assert!(put_claim(&grant).is_err(), "accepted {bad}");
        }
        grant.filename = "one.md".into();
        grant.expected_blake3 = "NOT_A_HASH".into();
        assert!(put_claim(&grant).is_err());
    }
}
