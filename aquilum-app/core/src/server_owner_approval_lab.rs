//! B1b1b laboratory: DETACHED, operator-signed approval bound to an inert
//! installation proposal. This module DOES NOT perform installation,
//! persist trust, grant OS privileges or accept UI consent by itself.
//!
//! Three separate authorities MUST be distinguished:
//! (1) B1a release-signing key authenticates a package manifest.
//! (2) B0b SSH host key authenticates a target host.
//! (3) This separately pinned operator public key authenticates approval.
//!
//! This lab verifies a signature and constructs an inert compare-and-swap
//! request; it does NOT prove that an operator saw the correct screen or
//! that any durable external witness accepted the request. A stale or fake
//! external floor, compromised operator key, untrusted clock, hostile-root
//! file replacement or replay against a rolled-back witness remain unsafe.

use crate::server_release_floor_lab::{LabPlanBinding, ProposedFloor, TrustedFloorHead};
use crate::server_staging_lab::{ApprovedPlanDigest, InstallId, StageReceipt};
use ring::signature;

pub const MAX_APPROVAL_VALIDITY_SECONDS: u64 = 15 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExternalApprovalScope {
    /// Must originate from a separate, trusted operator/host authority.
    pub install_id: InstallId,
    pub approved_plan_digest: ApprovedPlanDigest,
    pub pinned_host_key_sha256: [u8; 32],
    pub independently_pinned_previous_head: TrustedFloorHead,
    /// Must be observed from a protected, independent CAS witness.
    pub expected_witness_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedApprovalClaim {
    pub approver_id: String,
    pub scope_binding: LabPlanBinding,
    pub previous_head: TrustedFloorHead,
    pub proposed_head: TrustedFloorHead,
    pub staged_package_sha256: [u8; 32],
    pub staged_package_size: u64,
    pub expected_witness_revision: u64,
    pub approval_nonce: [u8; 16],
    pub issued_at_unix: u64,
    pub expires_at_unix: u64,
}

impl SignedApprovalClaim {
    /// Canonical binary encoding. Every fixed-width integer is big-endian.
    /// No JSON, URL, shell, path, arbitrary file or string normalization.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        use crate::server_owner_approval_wire::{OwnerApprovalWireV1, OwnerFloorWire};
        let head = |head: TrustedFloorHead| OwnerFloorWire {
            generation: head.generation,
            sequence: head.floor.sequence,
            version: [
                head.floor.version.major, head.floor.version.minor,
                head.floor.version.patch,
            ],
            manifest_fingerprint: head.floor.signed_manifest_fingerprint,
            chain_tip: head.chain_tip,
        };
        OwnerApprovalWireV1 {
            approver_id: self.approver_id.clone(),
            install_id: self.scope_binding.install_id.0,
            plan_digest: self.scope_binding.plan_digest.0,
            ssh_host_sha256: self.scope_binding.pinned_host_key_sha256,
            manifest_fingerprint: self.scope_binding.signed_manifest_fingerprint,
            previous: head(self.previous_head),
            proposed: head(self.proposed_head),
            staged_package_sha256: self.staged_package_sha256,
            staged_package_size: self.staged_package_size,
            expected_witness_revision: self.expected_witness_revision,
            approval_nonce: self.approval_nonce,
            issued_at_unix: self.issued_at_unix,
            expires_at_unix: self.expires_at_unix,
        }.canonical_bytes()
    }

    fn validate_fields(&self, now_unix: u64) -> Result<(), ApprovalError> {
        let id = self.approver_id.as_bytes();
        if id.is_empty() || id.len() > 64
            || !id.iter().all(|b| b.is_ascii_alphanumeric()
                || matches!(*b, b'.' | b'-' | b'_'))
            || self.scope_binding.install_id.0 == [0; 16]
            || self.scope_binding.plan_digest.0 == [0; 32]
            || self.scope_binding.pinned_host_key_sha256 == [0; 32]
            || self.scope_binding.signed_manifest_fingerprint == [0; 32]
            || self.staged_package_sha256 == [0; 32]
            || self.staged_package_size == 0
            || self.approval_nonce == [0; 16]
        {
            return Err(ApprovalError::MalformedClaim);
        }
        let span = self.expires_at_unix.checked_sub(self.issued_at_unix)
            .ok_or(ApprovalError::MalformedClaim)?;
        if span == 0 || span > MAX_APPROVAL_VALIDITY_SECONDS {
            return Err(ApprovalError::MalformedClaim);
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedOwnerApproval {
    pub claim: SignedApprovalClaim,
    pub signature: [u8; 64],
}

/// The caller MUST pin this key using a separate policy/key ceremony.
/// It must never come from the candidate package or SSH peer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedOwnerApprover {
    pub approver_id: String,
    pub public_key_ed25519: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InertWitnessCasRequest {
    pub expected_revision: u64,
    pub desired_revision: u64,
    pub previous_head: TrustedFloorHead,
    pub proposed_head: TrustedFloorHead,
    pub install_id: InstallId,
    pub approved_plan_digest: ApprovedPlanDigest,
    pub approval_nonce: [u8; 16],
    /// The independent witness still has to check and persist this CAS.
    /// Returning this struct is NEVER authority to execute a package.
    pub requires_external_commit: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalError {
    MalformedClaim,
    NotYetValid,
    Expired,
    UntrustedApprover,
    BadSignature,
    ScopeMismatch,
    WitnessRevisionMismatch,
    ProposalMismatch,
    StageMismatch,
    InvalidTransition,
}

/// A read-only verifier. It neither writes a witness nor dispatches actions.
/// A signed, exact approval plus independently pinned scope is necessary but
/// NOT sufficient to run a binary. The external CAS and activation authority
/// must be separately implemented and tested.
pub fn verify_owner_approval(
    signed: &SignedOwnerApproval,
    trusted: &TrustedOwnerApprover,
    scope: ExternalApprovalScope,
    staged: &StageReceipt,
    proposed: ProposedFloor,
    now_unix: u64,
) -> Result<InertWitnessCasRequest, ApprovalError> {
    let claim = &signed.claim;
    claim.validate_fields(now_unix)?;
    if trusted.approver_id != claim.approver_id
        || trusted.public_key_ed25519 == [0; 32]
    {
        return Err(ApprovalError::UntrustedApprover);
    }
    signature::UnparsedPublicKey::new(&signature::ED25519, &trusted.public_key_ed25519)
        .verify(&claim.canonical_bytes(), &signed.signature)
        .map_err(|_| ApprovalError::BadSignature)?;

    if scope.install_id.0 == [0; 16]
        || scope.approved_plan_digest.0 == [0; 32]
        || scope.pinned_host_key_sha256 == [0; 32]
        || claim.scope_binding.install_id != scope.install_id
        || claim.scope_binding.plan_digest != scope.approved_plan_digest
        || claim.scope_binding.pinned_host_key_sha256 != scope.pinned_host_key_sha256
        || claim.previous_head != scope.independently_pinned_previous_head
    {
        return Err(ApprovalError::ScopeMismatch);
    }
    if claim.expected_witness_revision != scope.expected_witness_revision {
        return Err(ApprovalError::WitnessRevisionMismatch);
    }
    if staged.install_id != scope.install_id
        || staged.approved_plan_digest != scope.approved_plan_digest
        || claim.scope_binding.signed_manifest_fingerprint
            != staged.verified_package.signed_manifest_fingerprint
        || claim.staged_package_sha256 != staged.verified_package.package_sha256
        || claim.staged_package_size != staged.verified_package.byte_length
        || claim.proposed_head.floor != staged.verified_package.proposed_floor
    {
        return Err(ApprovalError::StageMismatch);
    }
    if !proposed.requires_external_witness
        || claim.proposed_head != proposed.proposed_head
    {
        return Err(ApprovalError::ProposalMismatch);
    }
    let previous = scope.independently_pinned_previous_head;
    let next = proposed.proposed_head;
    if previous.generation != scope.expected_witness_revision
        || next.generation != previous.generation.checked_add(1)
            .ok_or(ApprovalError::InvalidTransition)?
        || next.floor.sequence <= previous.floor.sequence
        || next.floor.version < previous.floor.version
        || next.floor.signed_manifest_fingerprint !=
            claim.scope_binding.signed_manifest_fingerprint
        || next.chain_tip == previous.chain_tip
        || next.chain_tip == [0; 32]
    {
        return Err(ApprovalError::InvalidTransition);
    }
    let desired = scope.expected_witness_revision.checked_add(1)
        .ok_or(ApprovalError::InvalidTransition)?;
    Ok(InertWitnessCasRequest {
        expected_revision: scope.expected_witness_revision,
        desired_revision: desired,
        previous_head: previous,
        proposed_head: next,
        install_id: scope.install_id,
        approved_plan_digest: scope.approved_plan_digest,
        approval_nonce: claim.approval_nonce,
        requires_external_commit: true,
    })
}
