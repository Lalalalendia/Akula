//! NODE-E2E-01b: owner-signed, device-proved *read-only laboratory* adapter
//! around the existing NODE-E2E-01a verified Sync DAG bytes.
//!
//! Independent host trust, fresh challenge, current epoch and exact signed
//! claim digest MUST originate OUTSIDE the request and this module.
//! No enrollment/pairing/key storage, no network, no File Gate write, no
//! permission to stage or publish. This is not STY-DEVICE-01a roster adoption.

use crate::node_replica_lab::{verify_local_replica, OperatorFixtureAdmission};
use crate::sync::{DeviceId, DocumentId, RevisionId, SyncRelativePath, VaultId};
use ring::signature::{self, UnparsedPublicKey};
use std::path::Path;

const OWNER_DOMAIN: &[u8] = b"Akula/NODE-E2E-01b/owner-read-grant/v1\0";
const DEVICE_DOMAIN: &[u8] = b"Akula/NODE-E2E-01b/device-proof/v1\0";
const DIGEST_DOMAIN: &str = "Akula/NODE-E2E-01b/witnessed-owner-grant/v1";

/// An untrusted grant. The owner key MUST NOT be taken from this structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerReadGrant {
    pub vault_id: VaultId,
    pub document_id: DocumentId,
    pub revision_id: RevisionId,
    pub expected_blake3: String,
    pub filename: String,
    /// Exact host-approved Node identity, not an SSH address or a key ID.
    pub node_identity: [u8; 32],
    pub device_id: DeviceId,
    pub device_public_key: [u8; 32],
    /// Revocation/rollback fence backed by an *independent* trusted witness.
    pub grant_epoch: u64,
    /// Detached Ed25519 signature of owner_claim(grant).
    pub owner_signature: [u8; 64],
}

/// Inputs from an independently trusted host, never from the requesting peer.
/// Current epoch and digest are *not* self-authenticating: provisioned OS-owned
/// custody and persistent antirollback are separate production requirements.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndependentReadTrust {
    pub trusted_vault_id: VaultId,
    pub trusted_owner_public_key: [u8; 32],
    pub trusted_node_identity: [u8; 32],
    pub witnessed_current_epoch: u64,
    pub witnessed_exact_grant_digest: [u8; 32],
    /// Fresh nonzero challenge generated outside this verifier per session.
    pub current_session_challenge: [u8; 32],
}

/// Proof-of-possession for the exact device key bound into the owner's grant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceReadProof {
    pub device_signature: [u8; 64],
}

/// One opaque response for bad grant, stale epoch, absent object or bad bytes.
/// No existence disclosure or plaintext on any failure path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadDenied;

/// Canonical signed owner intent, with strict length and UTF-8 path rules.
/// This is a bounded fixed-order byte format, not an ambiguous JSON signer.
pub fn owner_claim(grant: &OwnerReadGrant) -> Result<Vec<u8>, ReadDenied> {
    let path = SyncRelativePath::parse(&grant.filename).map_err(|_| ReadDenied)?;
    let name = grant.filename.as_bytes();
    let digest = grant.expected_blake3.as_bytes();
    if grant.vault_id.as_uuid().is_nil()
        || grant.document_id.as_uuid().is_nil()
        || grant.revision_id.as_uuid().is_nil()
        || grant.device_id.as_uuid().is_nil()
        || grant.node_identity == [0; 32]
        || grant.device_public_key == [0; 32]
        || grant.grant_epoch == 0
        || name.is_empty() || name.len() > 255
        || path.as_str() != grant.filename
        || !path.as_str().ends_with(".md")
        || path.as_str().contains('/')
        || digest.len() != 64
        || !digest.iter().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
    {
        return Err(ReadDenied);
    }
    let mut out = Vec::with_capacity(OWNER_DOMAIN.len() + 16 * 4 + 32 * 2 + 8 + 64 + 2 + name.len());
    out.extend_from_slice(OWNER_DOMAIN);
    out.extend_from_slice(grant.vault_id.as_uuid().as_bytes());
    out.extend_from_slice(grant.document_id.as_uuid().as_bytes());
    out.extend_from_slice(grant.revision_id.as_uuid().as_bytes());
    out.extend_from_slice(grant.device_id.as_uuid().as_bytes());
    out.extend_from_slice(&grant.node_identity);
    out.extend_from_slice(&grant.device_public_key);
    out.extend_from_slice(&grant.grant_epoch.to_be_bytes());
    out.extend_from_slice(digest);
    out.extend_from_slice(&(name.len() as u16).to_be_bytes());
    out.extend_from_slice(name);
    Ok(out)
}

/// Locally computed digest for comparison with a separately witnessed EXACT
/// grant digest. Computing a digest from the request and calling it a witness
/// would be a security bug; the caller must pin this value independently.
pub fn signed_grant_digest(grant: &OwnerReadGrant) -> Result<[u8; 32], ReadDenied> {
    let mut hash = blake3::Hasher::new_derive_key(DIGEST_DOMAIN);
    hash.update(&owner_claim(grant)?);
    hash.update(&grant.owner_signature);
    Ok(*hash.finalize().as_bytes())
}

/// Device signs this after receiving a new independent challenge from the
/// already authenticated Node. Reusing a challenge requires caller rejection.
pub fn device_challenge_claim(
    grant: &OwnerReadGrant, challenge: [u8; 32],
) -> Result<Vec<u8>, ReadDenied> {
    if challenge == [0; 32] { return Err(ReadDenied); }
    let digest = signed_grant_digest(grant)?;
    let mut out = Vec::with_capacity(DEVICE_DOMAIN.len() + 32 * 3);
    out.extend_from_slice(DEVICE_DOMAIN);
    out.extend_from_slice(&grant.node_identity);
    out.extend_from_slice(&digest);
    out.extend_from_slice(&challenge);
    Ok(out)
}

/// Cryptographic + witnessed read gate before calling existing Sync DAG
/// verified readback. No raw File Gate or absolute-path API is introduced.
/// Production MUST supply trust and one-use challenge from independently
/// authenticated Node/session/OS custody and recheck latest revoke at read.
pub fn read_with_device_proof(
    state_dir: &Path,
    grant: &OwnerReadGrant,
    trust: &IndependentReadTrust,
    proof: &DeviceReadProof,
) -> Result<Vec<u8>, ReadDenied> {
    let owner_bytes = owner_claim(grant)?;
    if trust.trusted_vault_id != grant.vault_id
        || trust.trusted_owner_public_key == [0; 32]
        || trust.trusted_node_identity == [0; 32]
        || trust.trusted_node_identity != grant.node_identity
        || trust.witnessed_current_epoch == 0
        || trust.witnessed_current_epoch != grant.grant_epoch
        || trust.witnessed_exact_grant_digest != signed_grant_digest(grant)?
        || trust.current_session_challenge == [0; 32]
    {
        return Err(ReadDenied);
    }
    UnparsedPublicKey::new(&signature::ED25519, &trust.trusted_owner_public_key)
        .verify(&owner_bytes, &grant.owner_signature)
        .map_err(|_| ReadDenied)?;
    let proof_bytes = device_challenge_claim(grant, trust.current_session_challenge)?;
    UnparsedPublicKey::new(&signature::ED25519, &grant.device_public_key)
        .verify(&proof_bytes, &proof.device_signature)
        .map_err(|_| ReadDenied)?;

    let admission = OperatorFixtureAdmission {
        vault_id: grant.vault_id,
        document_id: grant.document_id,
        revision_id: grant.revision_id,
        filename: grant.filename.clone(),
        expected_blake3: grant.expected_blake3.clone(),
    };
    verify_local_replica(state_dir, &admission).map_err(|_| ReadDenied)
}
