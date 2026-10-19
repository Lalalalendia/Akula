//! RN-4B/APP-GRANT-01: strictly read-only authorization preflight.
//!
//! This is a laboratory bridge between an authenticated transport EndpointId,
//! an Akula-owned DeviceId/VaultId policy snapshot, and the existing File Gate.
//! Only the local application may supply LocalPolicy and authenticated_endpoint.
//! Network data MUST NOT create or update a LocalPolicy.
//!
//! The module does not implement enrollment, persistent revocation, a canonical
//! remote grant wire protocol, staging, Bao verification, durable receipts,
//! session freshness or any production Akula synchronization capability.

use ed25519_dalek::{Signature, VerifyingKey};
use std::path::Path;

const DOMAIN: &[u8] = b"akula/staya/app-grant-01/read-only/v1\0";
const MAX_LAB_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RequestedOperation {
    Pull = 1,
    Push = 2,
    Delete = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerBinding {
    /// The real caller must get this from QUIC Connection::remote_id().
    pub endpoint_id: [u8; 32],
    pub device_id: [u8; 16],
    pub vault_id: [u8; 16],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectManifest {
    pub blake3: [u8; 32],
    pub byte_len: u64,
}

impl ObjectManifest {
    pub fn for_bytes(bytes: &[u8]) -> Self {
        Self {
            blake3: *blake3::hash(bytes).as_bytes(),
            byte_len: bytes.len() as u64,
        }
    }
}

/// A signed, fixed-layout proposal. Signature verification never enrolls a peer
/// or bypasses locally issued Akula policy. There is no network deserializer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GrantScope {
    pub binding: PeerBinding,
    pub object: ObjectManifest,
    pub policy_epoch: u64,
    pub expires_at: u64,
    pub nonce: [u8; 16],
    pub operation: RequestedOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SignedGrant {
    pub scope: GrantScope,
    pub signature: [u8; 64],
}

impl SignedGrant {
    fn signing_material(&self) -> Vec<u8> {
        let p = &self.scope;
        let mut out = Vec::with_capacity(DOMAIN.len() + 32 + 16 + 16 + 32 + 8 * 3 + 16 + 1);
        out.extend_from_slice(DOMAIN);
        out.extend_from_slice(&p.binding.endpoint_id);
        out.extend_from_slice(&p.binding.device_id);
        out.extend_from_slice(&p.binding.vault_id);
        out.extend_from_slice(&p.object.blake3);
        out.extend_from_slice(&p.object.byte_len.to_be_bytes());
        out.extend_from_slice(&p.policy_epoch.to_be_bytes());
        out.extend_from_slice(&p.expires_at.to_be_bytes());
        out.extend_from_slice(&p.nonce);
        out.push(p.operation as u8);
        out
    }
}

/// Trusted *local* policy snapshot provided by Akula Core, never peer input.
/// Production needs persistent, atomic policy epochs and revocation fencing.
#[derive(Clone, Debug)]
pub struct LocalPolicy {
    pub trusted_issuer: VerifyingKey,
    pub binding: Option<PeerBinding>,
    pub approved_object: Option<ObjectManifest>,
    pub epoch: u64,
    pub expected_nonce: [u8; 16],
    pub revoked: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionError {
    InvalidSignature,
    UnknownOrRevokedPeer,
    WrongPeerBinding,
    WrongVaultOrDevice,
    StalePolicy,
    ExpiredGrant,
    WrongNonce,
    UnapprovedObject,
    UnsupportedOperation,
    FileGateRejected,
}

/// Only Pull is considered. Signed Push and Delete commands must fail closed.
/// The caller MUST pass the authenticated QUIC remote ID and a local Core
/// policy snapshot. No receipt is produced, and no policy is persisted here.
pub fn authorize_pull(
    policy: &LocalPolicy,
    actual_remote_endpoint: [u8; 32],
    request_object: ObjectManifest,
    request_operation: RequestedOperation,
    signed: &SignedGrant,
    trusted_now: u64,
) -> Result<(), AdmissionError> {
    let signature = Signature::from_bytes(&signed.signature);
    policy
        .trusted_issuer
        .verify_strict(&signed.signing_material(), &signature)
        .map_err(|_| AdmissionError::InvalidSignature)?;

    let scope = &signed.scope;
    if request_operation != RequestedOperation::Pull || scope.operation != RequestedOperation::Pull {
        return Err(AdmissionError::UnsupportedOperation);
    }
    if policy.revoked || policy.binding.is_none() {
        return Err(AdmissionError::UnknownOrRevokedPeer);
    }
    if scope.binding.endpoint_id != actual_remote_endpoint {
        return Err(AdmissionError::WrongPeerBinding);
    }
    if Some(scope.binding) != policy.binding {
        return Err(AdmissionError::WrongVaultOrDevice);
    }
    if policy.epoch == 0 || scope.policy_epoch != policy.epoch {
        return Err(AdmissionError::StalePolicy);
    }
    if scope.expires_at == 0 || trusted_now >= scope.expires_at {
        return Err(AdmissionError::ExpiredGrant);
    }
    if scope.nonce != policy.expected_nonce {
        return Err(AdmissionError::WrongNonce);
    }
    if scope.object.byte_len > MAX_LAB_BYTES
        || Some(scope.object) != policy.approved_object
        || scope.object != request_object
    {
        return Err(AdmissionError::UnapprovedObject);
    }
    Ok(())
}

/// Lab-only proof that denial occurs BEFORE the existing Core File Gate.
/// The fixture is pre-known local TEXT; no incoming network bytes reach a
/// filesystem. This does not create an Akula authorized remote apply API.
pub fn guarded_fixture_filegate_publish(
    base: &Path,
    tag: u8,
    policy: &LocalPolicy,
    actual_remote_endpoint: [u8; 32],
    request_object: ObjectManifest,
    request_operation: RequestedOperation,
    signed: &SignedGrant,
    trusted_now: u64,
) -> Result<(), AdmissionError> {
    authorize_pull(
        policy,
        actual_remote_endpoint,
        request_object,
        request_operation,
        signed,
        trusted_now,
    )?;
    if request_object != ObjectManifest::for_bytes(crate::TEXT.as_bytes()) {
        return Err(AdmissionError::UnapprovedObject);
    }
    crate::filegate_publish(base, tag).map_err(|_| AdmissionError::FileGateRejected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn fixture() -> (SigningKey, LocalPolicy, SignedGrant, ObjectManifest) {
        // Deterministic TEST signer. Keys in production must be managed by Core.
        let signer = SigningKey::from_bytes(&[47u8; 32]);
        let binding = PeerBinding {
            endpoint_id: [21; 32],
            device_id: [33; 16],
            vault_id: [55; 16],
        };
        let object = ObjectManifest::for_bytes(crate::TEXT.as_bytes());
        let scope = GrantScope {
            binding,
            object,
            policy_epoch: 7,
            expires_at: 200,
            nonce: [88; 16],
            operation: RequestedOperation::Pull,
        };
        let mut grant = SignedGrant {
            scope,
            signature: [0; 64],
        };
        grant.signature = signer.sign(&grant.signing_material()).to_bytes();
        let policy = LocalPolicy {
            trusted_issuer: signer.verifying_key(),
            binding: Some(binding),
            approved_object: Some(object),
            epoch: 7,
            expected_nonce: [88; 16],
            revoked: false,
        };
        (signer, policy, grant, object)
    }

    fn re_sign(signer: &SigningKey, grant: &mut SignedGrant) {
        grant.signature = signer.sign(&grant.signing_material()).to_bytes();
    }

    #[test]
    fn only_signed_exact_read_scope_can_reach_real_file_gate() {
        let (_, policy, grant, object) = fixture();
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            guarded_fixture_filegate_publish(
                temp.path(), 1, &policy, [21; 32], object,
                RequestedOperation::Pull, &grant, 100,
            ),
            Ok(())
        );
        assert_eq!(std::fs::read(crate::file_path(temp.path(), 1)).unwrap(), crate::TEXT.as_bytes());
        assert_eq!(
            crate::admit_after_restart(temp.path(), 1),
            Err(crate::ReceiptError::MissingReceipt),
            "a network grant plus verified local bytes are not a durable receipt"
        );
        assert_eq!(
            guarded_fixture_filegate_publish(
                temp.path(), 1, &policy, [21; 32], object,
                RequestedOperation::Pull, &grant, 100,
            ),
            Err(AdmissionError::FileGateRejected),
            "no overwrite/replay as a new filename"
        );
    }

    #[test]
    fn bad_signature_or_rewritten_scope_never_creates_a_file() {
        let (signer, policy, mut grant, object) = fixture();
        let temp = tempfile::tempdir().unwrap();
        grant.scope.binding.vault_id[0] ^= 1; // modified without resigning
        assert_eq!(
            guarded_fixture_filegate_publish(
                temp.path(), 1, &policy, [21; 32], object,
                RequestedOperation::Pull, &grant, 100,
            ),
            Err(AdmissionError::InvalidSignature)
        );
        assert!(!crate::file_path(temp.path(), 1).exists());
        grant.scope.binding.vault_id[0] ^= 1;
        grant.signature[0] ^= 1;
        assert_eq!(
            authorize_pull(&policy, [21; 32], object, RequestedOperation::Pull, &grant, 100),
            Err(AdmissionError::InvalidSignature)
        );
        let (_, mut policy, grant, _) = fixture();
        policy.trusted_issuer = SigningKey::from_bytes(&[99u8; 32]).verifying_key();
        assert_eq!(
            authorize_pull(&policy, [21; 32], object, RequestedOperation::Pull, &grant, 100),
            Err(AdmissionError::InvalidSignature)
        );
        let _ = signer;
    }

    #[test]
    fn untrusted_peer_revoked_device_wrong_vault_and_stale_policy_all_denied_pre_gate() {
        let (signer, policy, grant, object) = fixture();
        let mut trials = Vec::new();
        let mut p = policy.clone();
        p.binding = None;
        trials.push((p, grant, [21; 32], object, RequestedOperation::Pull, 100, AdmissionError::UnknownOrRevokedPeer));

        let mut p = policy.clone();
        p.revoked = true;
        trials.push((p, grant, [21; 32], object, RequestedOperation::Pull, 100, AdmissionError::UnknownOrRevokedPeer));

        trials.push((policy.clone(), grant, [22; 32], object, RequestedOperation::Pull, 100, AdmissionError::WrongPeerBinding));

        let mut g = grant;
        g.scope.binding.device_id[0] ^= 1;
        re_sign(&signer, &mut g);
        trials.push((policy.clone(), g, [21; 32], object, RequestedOperation::Pull, 100, AdmissionError::WrongVaultOrDevice));

        let mut g = grant;
        g.scope.binding.vault_id[0] ^= 1;
        re_sign(&signer, &mut g);
        trials.push((policy.clone(), g, [21; 32], object, RequestedOperation::Pull, 100, AdmissionError::WrongVaultOrDevice));

        let mut p = policy.clone();
        p.epoch = 8; // revoke by bumping local policy generation
        trials.push((p, grant, [21; 32], object, RequestedOperation::Pull, 100, AdmissionError::StalePolicy));

        let mut p = policy.clone();
        p.expected_nonce[0] ^= 1;
        trials.push((p, grant, [21; 32], object, RequestedOperation::Pull, 100, AdmissionError::WrongNonce));

        let mut p = policy.clone();
        p.approved_object = None;
        trials.push((p, grant, [21; 32], object, RequestedOperation::Pull, 100, AdmissionError::UnapprovedObject));

        let mut other = object;
        other.blake3[0] ^= 1;
        trials.push((policy.clone(), grant, [21; 32], other, RequestedOperation::Pull, 100, AdmissionError::UnapprovedObject));

        trials.push((policy.clone(), grant, [21; 32], object, RequestedOperation::Pull, 200, AdmissionError::ExpiredGrant));

        let mut g = grant;
        g.scope.operation = RequestedOperation::Push;
        re_sign(&signer, &mut g);
        trials.push((policy.clone(), g, [21; 32], object, RequestedOperation::Push, 100, AdmissionError::UnsupportedOperation));

        let mut g = grant;
        g.scope.operation = RequestedOperation::Delete;
        re_sign(&signer, &mut g);
        trials.push((policy.clone(), g, [21; 32], object, RequestedOperation::Delete, 100, AdmissionError::UnsupportedOperation));

        let mut g = grant;
        g.scope.object.byte_len += 1;
        re_sign(&signer, &mut g);
        trials.push((policy, g, [21; 32], object, RequestedOperation::Pull, 100, AdmissionError::UnapprovedObject));

        assert_eq!(trials.len(), 13);
        for (i, (p, signed, endpoint, requested, operation, now, expected)) in trials.into_iter().enumerate() {
            let temp = tempfile::tempdir().unwrap();
            assert_eq!(
                guarded_fixture_filegate_publish(
                    temp.path(), 1, &p, endpoint, requested,
                    operation, &signed, now,
                ),
                Err(expected),
                "case {i}"
            );
            assert!(!crate::file_path(temp.path(), 1).exists(), "case {i} created an unexpected file");
            assert!(!crate::receipt_path(temp.path(), 1).exists(), "case {i} created a receipt");
        }
    }

    #[test]
    fn manifest_exceeding_bound_is_denied_even_if_trusted_policy_and_signature_match() {
        let (signer, mut policy, mut grant, _) = fixture();
        let oversized = ObjectManifest {
            blake3: [1; 32],
            byte_len: MAX_LAB_BYTES + 1,
        };
        grant.scope.object = oversized;
        re_sign(&signer, &mut grant);
        policy.approved_object = Some(oversized);
        assert_eq!(
            authorize_pull(&policy, [21; 32], oversized, RequestedOperation::Pull, &grant, 100),
            Err(AdmissionError::UnapprovedObject)
        );
    }
}
