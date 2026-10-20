//! B1b1a: laboratory proof of owner-signed install approval.
//!
//! This is deliberately an inert, opt-in validation layer. It does NOT confer
//! permission to execute a staged artifact, persist a rollback floor, install
//! systemd services, access SSH credentials, or write to user vaults.
//!
//! The installer must independently pin the operator's public key, read the
//! current host-owned rollback floor, verify the real SSH host identity, and
//! reopen/reverify the staged inode. No such trusted installer exists here.

use crate::server_release_lab::{ReleaseFloor, VerifiedRelease};
use crate::server_staging_lab::{ApprovedPlanDigest, InstallId, StageReceipt};
use ring::signature;

const APPROVAL_DOMAIN: &[u8] = b"Akula Node B1b1a owner approval v1\0";
const APPROVAL_FINGERPRINT_DOMAIN: &str = "Akula B1b1a owner approval witness v1";
const MAX_APPROVAL_LIFETIME: u64 = 15 * 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperatorApprovalV1 {
    pub operator_id: String,
    pub install_id: InstallId,
    pub host_key_sha256: [u8; 32],
    pub plan_digest: ApprovedPlanDigest,
    pub package_sha256: [u8; 32],
    pub signed_manifest_fingerprint: [u8; 32],
    pub release_sequence: u64,
    pub issued_at_unix: u64,
    pub expires_at_unix: u64,
}

impl OperatorApprovalV1 {
    /// Fixed-order, domain-separated bytes: independent of JSON, locale or paths.
    pub fn canonical_signed_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(256);
        out.extend_from_slice(APPROVAL_DOMAIN);
        out.push(1); // schema version
        out.push(u8::try_from(self.operator_id.len()).unwrap_or(u8::MAX));
        out.extend_from_slice(self.operator_id.as_bytes());
        out.extend_from_slice(&self.install_id.0);
        out.extend_from_slice(&self.host_key_sha256);
        out.extend_from_slice(&self.plan_digest.0);
        out.extend_from_slice(&self.package_sha256);
        out.extend_from_slice(&self.signed_manifest_fingerprint);
        out.extend_from_slice(&self.release_sequence.to_be_bytes());
        out.extend_from_slice(&self.issued_at_unix.to_be_bytes());
        out.extend_from_slice(&self.expires_at_unix.to_be_bytes());
        out
    }

    fn validate(&self, now_unix: u64) -> Result<(), ApprovalError> {
        let id = self.operator_id.as_bytes();
        if id.is_empty() || id.len() > 64
            || !id.iter().all(|b| b.is_ascii_alphanumeric()
                || *b == b'-' || *b == b'_' || *b == b'.')
            || self.install_id.0.iter().all(|b| *b == 0)
            || self.host_key_sha256.iter().all(|b| *b == 0)
            || self.plan_digest.0.iter().all(|b| *b == 0)
            || self.package_sha256.iter().all(|b| *b == 0)
            || self.signed_manifest_fingerprint.iter().all(|b| *b == 0)
            || self.release_sequence == 0
        {
            return Err(ApprovalError::InvalidMetadata);
        }
        let lifetime = self.expires_at_unix.checked_sub(self.issued_at_unix)
            .ok_or(ApprovalError::InvalidMetadata)?;
        if self.issued_at_unix == 0 || lifetime == 0 || lifetime > MAX_APPROVAL_LIFETIME {
            return Err(ApprovalError::InvalidMetadata);
        }
        if now_unix < self.issued_at_unix {
            return Err(ApprovalError::NotYetValid);
        }
        if now_unix >= self.expires_at_unix {
            return Err(ApprovalError::Expired);
        }
        Ok(())
    }
}

/// The independent, host-owned operator trust anchor is an external input.
/// Never derive this key from candidate bytes, the SSH server, or the plan.
#[derive(Clone, Debug)]
pub struct TrustedOperatorKey {
    pub operator_id: String,
    pub ed25519_public: [u8; 32],
}

#[derive(Clone, Debug)]
pub struct SignedOperatorApproval {
    pub document: OperatorApprovalV1,
    pub signature: [u8; 64],
}

/// Observed host identity, trusted dry-run digest and floor must each come
/// from independent installer-owned authority, not from a package or HTTP.
#[derive(Clone, Copy)]
pub struct ApprovalContext<'a> {
    pub verified_release: &'a VerifiedRelease,
    pub staged_receipt: &'a StageReceipt,
    pub observed_host_key_sha256: [u8; 32],
    pub trusted_plan_digest: ApprovedPlanDigest,
    pub persisted_floor: ReleaseFloor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalError {
    InvalidMetadata,
    NotYetValid,
    Expired,
    UntrustedOperator,
    BadSignature,
    ScopeMismatch,
    RollbackRefused,
}

/// Read-only witness of matching input proofs; NOT an activation capability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedApproval {
    pub install_id: InstallId,
    pub approved_plan_digest: ApprovedPlanDigest,
    pub host_key_sha256: [u8; 32],
    pub proposed_floor: ReleaseFloor,
    pub signed_approval_fingerprint: [u8; 32],
}

pub fn verify_operator_approval(
    signed: &SignedOperatorApproval,
    trusted: &TrustedOperatorKey,
    ctx: ApprovalContext<'_>,
    now_unix: u64,
) -> Result<VerifiedApproval, ApprovalError> {
    signed.document.validate(now_unix)?;
    if signed.document.operator_id != trusted.operator_id
        || trusted.ed25519_public.iter().all(|b| *b == 0)
    {
        return Err(ApprovalError::UntrustedOperator);
    }
    let signed_bytes = signed.document.canonical_signed_bytes();
    signature::UnparsedPublicKey::new(&signature::ED25519, trusted.ed25519_public)
        .verify(&signed_bytes, &signed.signature)
        .map_err(|_| ApprovalError::BadSignature)?;

    let document = &signed.document;
    let staged = ctx.staged_receipt;
    let manifest = ctx.verified_release.manifest();
    let candidate = ctx.verified_release.release_floor_candidate();
    let recorded = &staged.verified_package;

    // Match both the signed owner consent and the independent staged/release
    // receipts. Neither a StageReceipt nor a hash alone proves a sealed inode.
    if document.install_id != staged.install_id
        || document.plan_digest != ctx.trusted_plan_digest
        || document.plan_digest != staged.approved_plan_digest
        || document.host_key_sha256 != ctx.observed_host_key_sha256
        || document.package_sha256 != manifest.package_sha256
        || document.package_sha256 != recorded.package_sha256
        || document.signed_manifest_fingerprint != candidate.signed_manifest_fingerprint
        || document.signed_manifest_fingerprint != recorded.signed_manifest_fingerprint
        || document.release_sequence != manifest.release_sequence
        || recorded.byte_length != manifest.package_size
        || recorded.proposed_floor != candidate
    {
        return Err(ApprovalError::ScopeMismatch);
    }

    // Recheck a freshly observed floor even when B1a had verified the release
    // with an older floor. No update or persistence is done in this module.
    if candidate.sequence < ctx.persisted_floor.sequence
        || candidate.version < ctx.persisted_floor.version
        || (candidate.sequence == ctx.persisted_floor.sequence
            && candidate.signed_manifest_fingerprint
                != ctx.persisted_floor.signed_manifest_fingerprint)
    {
        return Err(ApprovalError::RollbackRefused);
    }

    let mut hasher = blake3::Hasher::new_derive_key(APPROVAL_FINGERPRINT_DOMAIN);
    hasher.update(&signed_bytes);
    hasher.update(&signed.signature);
    Ok(VerifiedApproval {
        install_id: document.install_id,
        approved_plan_digest: document.plan_digest,
        host_key_sha256: document.host_key_sha256,
        proposed_floor: candidate,
        signed_approval_fingerprint: *hasher.finalize().as_bytes(),
    })
}
