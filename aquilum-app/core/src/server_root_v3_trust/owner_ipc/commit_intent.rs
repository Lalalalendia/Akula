//! B1-E2E-07: bounded, canonical owner+plan IPC COMMIT-INTENT review.
//!
//! This is NOT a commit opcode. The shipping candidate still reads only its
//! original preview protocol and exits HOLD. In particular, this module has
//! no listener, arbitrary path, /opt writer, systemd call or B1d0 CAS call.
//! It prepares verified, inert input for a later separately approved dispatch.
use super::*;
use crate::server_bootstrap_lab::DryRunPlan;
use crate::server_installer_os_witness_lab::OsWitnessLab;
use crate::server_owner_approval_wire::decode::SignedOwnerWireFrame;
use crate::server_witness_lab::WitnessCheckpoint;
use std::io::Read;

mod cas_inputs;

const INTENT_DOMAIN: &[u8] = b"AKULA-B1-E2E-07-OWNER-PLAN-COMMIT-INTENT-V1\0";
const MAX_PLAN_BYTES: usize = 16 * 1024;
const MAX_OWNER_BYTES: usize = 768;

#[allow(dead_code)]
#[derive(Debug)]
pub(super) struct InertRootCommitIntent {
    /// Strict canonical B0 v3, checked against the independently pinned
    /// release, OS, SSH identity, helper identity and owner signature.
    pub canonical_plan: Vec<u8>,
    pub signed_owner_frame: Vec<u8>,
    pub checked_scope: crate::server_root_v3_guard::CheckedRootV3Scope,
    /// The existing B1d0 witness/FD CAS must still be separately invoked.
    pub requires_independent_b1d0_cas: bool,
    /// Inputs derived exclusively from the verified owner claim, independently
    /// signed package FD and root-owned witness genesis; not client fields.
    pub checked_b1d0: cas_inputs::CheckedB1d0Inputs,
}

/// Exact single-request framing, independent of the older preview domain.
/// Both lengths are big-endian u16; no paths, commands, filenames, or
/// caller-provided root trust anchors are accepted in this framing.
fn read_bounded_intent(stream: &mut UnixStream) -> Result<(Vec<u8>, Vec<u8>), OwnerIpcError> {
    stream.set_read_timeout(Some(READ_DEADLINE))?;
    let mut header = vec![0u8; INTENT_DOMAIN.len() + 4];
    stream.read_exact(&mut header)?;
    if !header.starts_with(INTENT_DOMAIN) {
        return Err(OwnerIpcError::InvalidFrame);
    }
    let pos = INTENT_DOMAIN.len();
    let plan_len = u16::from_be_bytes([header[pos], header[pos + 1]]) as usize;
    let owner_len = u16::from_be_bytes([header[pos + 2], header[pos + 3]]) as usize;
    if !(2..=MAX_PLAN_BYTES).contains(&plan_len) || !(65..=MAX_OWNER_BYTES).contains(&owner_len) {
        return Err(OwnerIpcError::InvalidFrame);
    }
    let mut plan = vec![0u8; plan_len];
    let mut owner = vec![0u8; owner_len];
    stream.read_exact(&mut plan)?;
    stream.read_exact(&mut owner)?;
    // No pipelining, optional trailing bytes or requests that do not finish.
    let mut trailing = [0u8; 1];
    if stream.read(&mut trailing)? != 0 {
        return Err(OwnerIpcError::InvalidFrame);
    }
    Ok((plan, owner))
}

/// Reject even a *validly signed* owner frame when the separately supplied
/// B0 plan is not its exact canonical reviewed plan. The privileged verifier
/// independently rechecks the complete OS/release trust scope afterwards.
fn check_canonical_owner_binding(
    plan_bytes: &[u8],
    frame: &SignedOwnerWireFrame,
) -> Result<(), OwnerIpcError> {
    let plan: DryRunPlan =
        serde_json::from_slice(plan_bytes).map_err(|_| OwnerIpcError::InvalidFrame)?;
    if plan.schema_version != 3
        || !plan.verify_unchanged()
        || serde_json::to_vec(&plan).map_err(|_| OwnerIpcError::InvalidFrame)? != plan_bytes
    {
        return Err(OwnerIpcError::InvalidFrame);
    }
    let digest =
        blake3::Hash::from_hex(&plan.plan_blake3).map_err(|_| OwnerIpcError::InvalidFrame)?;
    let c = &frame.claim;
    if plan.install_id.as_bytes() != &c.install_id
        || digest.as_bytes() != &c.plan_digest
        || plan.target.trusted_key_sha256 != c.ssh_host_sha256
        || plan.package.package_sha256 != c.staged_package_sha256
    {
        return Err(OwnerIpcError::InvalidFrame);
    }
    Ok(())
}

/// Read-only root admission for a FUTURE fixed privileged dispatch. The
/// actual helper executable must already be independently signed, staged and
/// installed at the root-owned fixed inode. Absence is always HOLD. This
/// function is deliberately not called from the current root-helper main().
#[allow(dead_code)]
pub(super) fn prepare_provisioned_commit_intent_once(
    stream: &mut UnixStream,
) -> Result<InertRootCommitIntent, OwnerIpcError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(OwnerIpcError::Trust(RootEnrollmentError::NotRoot));
    }
    crate::server_root_v3_trust::inspect_provisioned_running_helper_hold()
        .map_err(|_| OwnerIpcError::InvalidFrame)?;
    let trust = open_fixed_trust_fd()?;
    let uid = read_allowed_peer(&trust)?;
    enrollment::fixed_service_account_matches(uid).map_err(OwnerIpcError::PeerEnrollment)?;
    let peer = verified_peer_uid(stream).map_err(OwnerIpcError::Peer)?;
    if peer == 0 || peer != uid {
        return Err(OwnerIpcError::WrongPeer);
    }
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| OwnerIpcError::Trust(RootEnrollmentError::InvalidOwnerClock))?
        .as_secs();
    let (canonical_plan, signed_owner_frame) = read_bounded_intent(stream)?;
    let owner =
        super::super::owner_frame::verify_from_trust_fd(&trust, &signed_owner_frame, now_unix)?;
    let frame = SignedOwnerWireFrame::decode(&signed_owner_frame)
        .map_err(RootEnrollmentError::OwnerFrame)?;
    check_canonical_owner_binding(&canonical_plan, &frame)?;
    let policy = RootEnrolledV3::from_trust_fd(&trust)?;
    // A signed plan is not a release signer or witness. Independently open
    // the registered B1a FD and current protected, monotonic witness state.
    let pinned = open_provisioned_first_b1a_release().map_err(OwnerIpcError::FirstRelease)?;
    let actor = OsWitnessLab::open_provisioned_first_install().map_err(OwnerIpcError::Witness)?;
    let observed = actor.current_checkpoint().map_err(OwnerIpcError::Witness)?;
    if observed != WitnessCheckpoint::genesis()
        || owner.claim.previous_head != observed.accepted_floor
        || owner.claim.expected_witness_revision != observed.revision
    {
        return Err(OwnerIpcError::InvalidFrame);
    }
    let (local_os, local_arch, systemd) = observed_root_host()?;
    let checked_scope = policy.verify_review(
        &canonical_plan,
        &frame.claim,
        &frame.signature,
        &pinned,
        observed.revision,
        local_os,
        local_arch,
        systemd,
        now_unix,
    )?;
    let checked_b1d0 = cas_inputs::bind_verified_b1d0_inputs(
        checked_scope,
        &owner,
        pinned.receipt.clone(),
        observed,
    )?;
    Ok(InertRootCommitIntent {
        canonical_plan,
        signed_owner_frame,
        checked_scope,
        checked_b1d0,
        requires_independent_b1d0_cas: true,
    })
}

#[cfg(test)]
mod tests;
