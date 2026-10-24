//! B1-E2E-07: strictly inert root-side B0 v3 and signed-owner correspondence.
//!
//! This does not create a privileged installer, enroll trust, start a service,
//! read SSH claims as authority, or advance the B1d0 witness. The independent
//! pins below MUST eventually be loaded from verified, root-owned enrollment
//! and an already B1a-verified Node release, never caller/SSH JSON.
use crate::server_bootstrap_lab::{
    DryRunPlan, MachineArch, SupportedOs,
};
use crate::server_owner_approval_wire::{OwnerApprovalWireV1, WireError};

const MAX_CANONICAL_PLAN_BYTES: usize = 16 * 1024;

/// In a production root helper these expected values MUST come from a
/// separate verified root trust record and the already pinned release FD.
/// Passing a candidate-supplied value is explicitly not authorization.
#[derive(Clone, Copy, Debug)]
pub struct RootIndependentPins<'a> {
    pub owner_id: &'a str,
    pub owner_ed25519: [u8; 32],
    pub ssh_host_sha256: [u8; 32],
    pub bootstrap_helper_sha256: [u8; 32],
    pub node_sha256: [u8; 32],
    pub node_size: u64,
    pub manifest_fingerprint: [u8; 32],
    pub release_signer_id: &'a str,
    pub release_version: &'a str,
    pub host_os: SupportedOs,
    pub host_arch: MachineArch,
    pub has_systemd: bool,
    pub witness_revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootPlanError {
    MalformedPlan,
    NoncanonicalPlan,
    WrongPlan,
    UntrustedRootScope,
    WrongOwnerScope,
    StaleWitness,
    OwnerSignature(WireError),
}

/// An observation of signature and scope consistency. It has no method for
/// opening a file, consuming CAS, executing an ELF or installing systemd.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct CheckedRootV3Scope {
    pub install_id: [u8; 16],
    pub plan_digest: [u8; 32],
    pub node_sha256: [u8; 32],
    pub bootstrap_helper_sha256: [u8; 32],
    pub approval_nonce: [u8; 16],
}

/// Verify the entire submitted B0 v3 plan, *not* just a caller-supplied
/// digest. Strict canonical JSON forbids unknown fields, alternate wire
/// encodings and ambiguity between what Windows displayed and root checks.
///
/// The returned scope is still inert. The existing B1d0 verifier MUST
/// independently check root-enrolled owner keys, release FD, floor, witness
/// and single-use nonce at the moment it commits.
pub fn verify_root_b0_v3_scope(
    plan_json: &[u8],
    owner: &OwnerApprovalWireV1,
    signature: &[u8; 64],
    pins: RootIndependentPins<'_>,
    now_unix: u64,
) -> Result<CheckedRootV3Scope, RootPlanError> {
    if plan_json.is_empty() || plan_json.len() > MAX_CANONICAL_PLAN_BYTES {
        return Err(RootPlanError::MalformedPlan);
    }
    let plan: DryRunPlan =
        serde_json::from_slice(plan_json).map_err(|_| RootPlanError::MalformedPlan)?;
    let canonical = serde_json::to_vec(&plan).map_err(|_| RootPlanError::MalformedPlan)?;
    if canonical != plan_json {
        return Err(RootPlanError::NoncanonicalPlan);
    }
    if plan.schema_version != 3
        || !plan.verify_unchanged()
        || plan.target.validate().is_err()
        || plan.package.validate().is_err()
        || plan.install_id.is_nil()
        || plan.inspected_host.cpu_count == 0
        || plan.inspected_host.ram_mib < 512
        || plan.inspected_host.free_disk_mib < 1024
        || !plan.inspected_host.has_systemd
        || !plan.inspected_host.has_sudo_rights
    {
        return Err(RootPlanError::WrongPlan);
    }
    // The signed preflight values alone never establish OS privileges.
    // Compare invariant platform facts and trust anchors against *independent*
    // root-enrolled state instead of trusting the SSH client's JSON.
    if !matches!(pins.host_os, SupportedOs::Ubuntu | SupportedOs::Debian)
        || pins.host_arch != MachineArch::X86_64
        || !pins.has_systemd
        || plan.inspected_host.os != pins.host_os
        || plan.inspected_host.arch != pins.host_arch
        || plan.target.trusted_key_sha256 != pins.ssh_host_sha256
        || pins.ssh_host_sha256 == [0; 32]
        || pins.bootstrap_helper_sha256 == [0; 32]
        || plan.bootstrap_helper_sha256 != Some(pins.bootstrap_helper_sha256)
        || pins.node_sha256 == [0; 32]
        || pins.node_size == 0
        || plan.package.package_sha256 != pins.node_sha256
        || pins.manifest_fingerprint == [0; 32]
        || plan.package.trusted_signer_id != pins.release_signer_id
        || plan.package.version != pins.release_version
        || pins.owner_id.is_empty()
        || pins.owner_ed25519 == [0; 32]
    {
        return Err(RootPlanError::UntrustedRootScope);
    }
    let plan_hash = blake3::Hash::from_hex(&plan.plan_blake3)
        .map_err(|_| RootPlanError::WrongPlan)?;
    if owner.install_id != *plan.install_id.as_bytes()
        || owner.plan_digest != *plan_hash.as_bytes()
        || owner.ssh_host_sha256 != pins.ssh_host_sha256
        || owner.staged_package_sha256 != pins.node_sha256
        || owner.staged_package_size != pins.node_size
        || owner.manifest_fingerprint != pins.manifest_fingerprint
        || owner.proposed.manifest_fingerprint != pins.manifest_fingerprint
        || owner.approver_id != pins.owner_id
    {
        return Err(RootPlanError::WrongOwnerScope);
    }
    if owner.expected_witness_revision != pins.witness_revision
        || owner.previous.generation != pins.witness_revision
        || owner.proposed.generation != pins.witness_revision.saturating_add(1)
        || pins.witness_revision == u64::MAX
    {
        return Err(RootPlanError::StaleWitness);
    }
    owner
        .verify_detached(signature, &pins.owner_ed25519, now_unix)
        .map_err(RootPlanError::OwnerSignature)?;
    Ok(CheckedRootV3Scope {
        install_id: owner.install_id,
        plan_digest: owner.plan_digest,
        node_sha256: owner.staged_package_sha256,
        bootstrap_helper_sha256: pins.bootstrap_helper_sha256,
        approval_nonce: owner.approval_nonce,
    })
}

#[cfg(test)]
mod tests;
