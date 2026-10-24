//! B1-E2E-02: immutable, signed-package-backed local review.
//!
//! This is neither a remote installation request nor an owner-signed B1b1b
//! approval. There are three separate proofs: the B0 read-only SSH result,
//! the independently signed B1a package, and a later durable owner signature.
//! Only the first two can be represented here. A click does not become a
//! privileged operation, systemd activation, or permission to access a vault.
use super::{
    verify_signed_release, ReleaseError, ReleaseFloor, SignedRelease, TrustedReleaseSigner,
};
use crate::server_bootstrap_lab::{DryRunPlan, MachineArch, PackagePin, ServerTarget, SupportedOs};
use std::io::Read;
use uuid::Uuid;

const REVIEW_TTL_SECONDS: u64 = 10 * 60;

/// No user-interface event can turn this result into a root authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewError {
    InvalidPreview,
    InvalidPackage,
    InvalidClock,
    EmptyNonce,
    SignedRelease(ReleaseError),
    StaleOrChanged,
    Expired,
    AlreadyUsed,
}
impl From<ReleaseError> for ReviewError {
    fn from(error: ReleaseError) -> Self {
        Self::SignedRelease(error)
    }
}

/// A single review screen bound to one actual B0 plan and independently
/// authenticated package bytes. The caller must obtain the original B0
/// observation through signed SSH KEX, not from a fabricated host response.
#[derive(Debug)]
pub struct PreparedReview {
    plan: DryRunPlan,
    plan_hash: [u8; 32],
    signed_manifest_fingerprint: [u8; 32],
    expires_at_unix: u64,
    created_at_unix: u64,
    nonce: [u8; 16],
    used: bool,
}

/// The user must echo exactly what was shown. An OK for a changed host,
/// package, release or plan is not transferable to the next candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewEcho {
    pub install_id: Uuid,
    pub plan_hash: [u8; 32],
    pub ssh_host_key_sha256: [u8; 32],
    pub package_sha256: [u8; 32],
    pub review_nonce: [u8; 16],
}

/// An ephemeral UI record, deliberately NOT SignedOwnerApproval, CAS witness,
/// installation command, secret, or activation authority. It is never enough
/// to authenticate a future SSH session or enable root installation.
#[must_use]
#[derive(Debug, PartialEq, Eq)]
pub struct InertUserConsent {
    install_id: Uuid,
    plan_hash: [u8; 32],
    ssh_host_key_sha256: [u8; 32],
    package_sha256: [u8; 32],
    manifest_fingerprint: [u8; 32],
    review_nonce: [u8; 16],
}
impl InertUserConsent {
    /// Fixed scope for a future independently signed owner claim.
    /// Returning these bytes alone MUST NOT grant installer permissions.
    pub fn scope(&self) -> ([u8; 16], [u8; 32], [u8; 32], [u8; 32], [u8; 32]) {
        (
            *self.install_id.as_bytes(),
            self.plan_hash,
            self.ssh_host_key_sha256,
            self.package_sha256,
            self.manifest_fingerprint,
        )
    }

    pub fn nonce(&self) -> [u8; 16] {
        self.review_nonce
    }

    /// A Windows owner signer may only sign an independently obtained B1b1b
    /// floor/receipt claim that agrees with the actual reviewed B0/B1a scope.
    /// This is a local scope check, NOT root witness authorization.
    pub fn matches_owner_claim(
        &self,
        claim: &crate::server_owner_approval_wire::OwnerApprovalWireV1,
    ) -> bool {
        claim.install_id == *self.install_id.as_bytes()
            && claim.plan_digest == self.plan_hash
            && claim.ssh_host_sha256 == self.ssh_host_key_sha256
            && claim.staged_package_sha256 == self.package_sha256
            && claim.manifest_fingerprint == self.manifest_fingerprint
            && claim.approval_nonce == self.review_nonce
            && claim.proposed.manifest_fingerprint == self.manifest_fingerprint
    }
}

fn hex_digest(value: &str) -> Result<[u8; 32], ReviewError> {
    if value.len() != 64 || !value.is_ascii() {
        return Err(ReviewError::InvalidPreview);
    }
    let mut result = [0; 32];
    for (i, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        fn digit(byte: u8) -> Option<u8> {
            match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                _ => None,
            }
        }
        result[i] = (digit(chunk[0]).ok_or(ReviewError::InvalidPreview)? << 4)
            | digit(chunk[1]).ok_or(ReviewError::InvalidPreview)?;
    }
    Ok(result)
}

fn preview_valid(plan: &DryRunPlan) -> bool {
    let facts = &plan.inspected_host;
    plan.verify_unchanged()
        && plan.target.validate().is_ok()
        && plan.package.validate().is_ok()
        && !plan.install_id.is_nil()
        && matches!(facts.os, SupportedOs::Ubuntu | SupportedOs::Debian)
        && facts.arch == MachineArch::X86_64
        && facts.has_systemd
        && facts.has_sudo_rights
        && facts.cpu_count != 0
        && facts.ram_mib >= 512
        && facts.free_disk_mib >= 1024
}

/// The actual package bytes are read and hashed by the existing B1a verifier.
/// No signer, manifest, minimum sequence or package hash comes from the VPS.
///
/// The preview must be built from a real independently pinned signed SSH
/// result. This does not re-authenticate SSH; later installer SSH must do
/// so again and root must independently verify owner signature and CAS.
pub fn prepare_signed_review<R: Read>(
    plan: DryRunPlan,
    release: &SignedRelease,
    trusted: &TrustedReleaseSigner,
    independently_stored_floor: ReleaseFloor,
    package: &mut R,
    now_unix: u64,
) -> Result<PreparedReview, ReviewError> {
    if !preview_valid(&plan) {
        return Err(ReviewError::InvalidPreview);
    }
    let hash = hex_digest(&plan.plan_blake3)?;
    let verified = verify_signed_release(release, trusted, independently_stored_floor, now_unix)?;
    let manifest = verified.manifest();
    // Do not permit a candidate manifest to relabel its signed identity as
    // the B0 package or to point at a different signed executable.
    let version = format!(
        "{}.{}.{}",
        manifest.version.major, manifest.version.minor, manifest.version.patch,
    );
    if plan.package.version != version
        || plan.package.trusted_signer_id != manifest.signer_id
        || plan.package.package_sha256 != manifest.package_sha256
    {
        return Err(ReviewError::InvalidPackage);
    }
    let exact_bytes = verified.verify_package_stream(package)?;
    let floor = verified.release_floor_candidate();
    if exact_bytes.signed_manifest_fingerprint != floor.signed_manifest_fingerprint
        || exact_bytes.package_sha256 != plan.package.package_sha256
        || exact_bytes.byte_length != manifest.package_size
    {
        return Err(ReviewError::InvalidPackage);
    }
    let expires_at_unix = now_unix
        .checked_add(REVIEW_TTL_SECONDS)
        .ok_or(ReviewError::InvalidClock)?
        .min(manifest.expires_unix);
    if expires_at_unix <= now_unix {
        return Err(ReviewError::Expired);
    }
    let nonce = *Uuid::new_v4().as_bytes();
    if nonce == [0; 16] {
        return Err(ReviewError::EmptyNonce);
    }
    Ok(PreparedReview {
        plan,
        plan_hash: hash,
        signed_manifest_fingerprint: floor.signed_manifest_fingerprint,
        expires_at_unix,
        created_at_unix: now_unix,
        nonce,
        used: false,
    })
}

impl PreparedReview {
    pub fn shown_plan(&self) -> &DryRunPlan {
        &self.plan
    }
    pub fn expires_at_unix(&self) -> u64 {
        self.expires_at_unix
    }

    /// Use this echo only after a user reviews the exact immutable screen.
    /// It has no privilege beyond this in-memory review session.
    pub fn expected_echo(&self) -> ReviewEcho {
        ReviewEcho {
            install_id: self.plan.install_id,
            plan_hash: self.plan_hash,
            ssh_host_key_sha256: self.plan.target.trusted_key_sha256,
            package_sha256: self.plan.package.package_sha256,
            review_nonce: self.nonce,
        }
    }

    pub fn cancel(&mut self) {
        self.used = true;
    }

    /// The current host and package must still match the signed SSH preview.
    /// A changed target or replay consumes the review attempt, never retries.
    pub fn consume_local_click(
        &mut self,
        echo: &ReviewEcho,
        current_host: &ServerTarget,
        current_package: &PackagePin,
        now_unix: u64,
    ) -> Result<InertUserConsent, ReviewError> {
        if self.used {
            return Err(ReviewError::AlreadyUsed);
        }
        self.used = true;
        if now_unix < self.created_at_unix {
            return Err(ReviewError::InvalidClock);
        }
        if now_unix >= self.expires_at_unix {
            return Err(ReviewError::Expired);
        }
        if !self.plan.verify_unchanged()
            || &self.plan.target != current_host
            || &self.plan.package != current_package
            || echo != &self.expected_echo()
        {
            return Err(ReviewError::StaleOrChanged);
        }
        Ok(InertUserConsent {
            install_id: self.plan.install_id,
            plan_hash: self.plan_hash,
            ssh_host_key_sha256: self.plan.target.trusted_key_sha256,
            package_sha256: self.plan.package.package_sha256,
            manifest_fingerprint: self.signed_manifest_fingerprint,
            review_nonce: self.nonce,
        })
    }
}

#[cfg(test)]
mod tests;
