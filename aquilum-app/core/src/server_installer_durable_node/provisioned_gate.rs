//! Root-owned production B0 v3 admission gate for the EXISTING B1d0 CAS.
//! This feature is non-shipping and does not add any remote install command.
//! The only root writer remains B1d0's fixed, locked and signed transaction.
use super::*;
use crate::server_owner_approval_wire::{OwnerApprovalWireV1, OwnerFloorWire};
use crate::server_root_v3_guard::CheckedRootV3Scope;
use crate::server_root_v3_trust::{
    load_provisioned_root_v3_policy, observed_root_host, open_provisioned_first_b1a_release,
};

/// Non-shipping root-only bridge for ONE Windows/Linux canonical signed
/// B1b1b frame. Root first authenticates the frame under the independently
/// enrolled owner identity, then delegates to the ORIGINAL B0v3/B1d0 writer.
/// Neither the frame nor this function is exposed over an SSH/IPC opcode.
#[cfg(feature = "server-root-b0v3-gate-lab")]
#[allow(dead_code, clippy::too_many_arguments)]
pub(crate) fn commit_first_provisioned_signed_frame(
    signed_frame: &[u8],
    canonical_b0_v3: &[u8],
    scope: ExternalApprovalScope,
    staged: &StageReceipt,
    proposed: ProposedFloor,
    expected_before: WitnessCheckpoint,
    challenge: [u8; 32],
) -> Result<DurableStatus, DurableError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(DurableError::NotRoot);
    }
    // Frame fields are not install authority. All expected OS, owner,
    // release, plan, CAS and witness identities are verified AGAIN by
    // commit_first_provisioned_opt before any file can be committed.
    let owner = crate::server_root_v3_trust::load_verified_owner_frame(signed_frame)
        .map_err(DurableError::OwnerReview)?;
    if !frame_matches_b1d0_inputs(&owner, scope, staged, proposed, expected_before) {
        return Err(DurableError::Unenrolled);
    }
    commit_first_provisioned_opt(
        &owner,
        canonical_b0_v3,
        scope,
        staged,
        proposed,
        expected_before,
        challenge,
    )
}

#[cfg(feature = "server-root-b0v3-gate-lab")]
fn frame_matches_b1d0_inputs(
    owner: &SignedOwnerApproval,
    scope: ExternalApprovalScope,
    staged: &StageReceipt,
    proposed: ProposedFloor,
    expected_before: WitnessCheckpoint,
) -> bool {
    let c = &owner.claim;
    c.scope_binding.install_id == scope.install_id
        && c.scope_binding.plan_digest == scope.approved_plan_digest
        && c.scope_binding.pinned_host_key_sha256 == scope.pinned_host_key_sha256
        && c.previous_head == scope.independently_pinned_previous_head
        && c.previous_head == expected_before.accepted_floor
        && c.expected_witness_revision == scope.expected_witness_revision
        && c.expected_witness_revision == expected_before.revision
        && c.proposed_head == proposed.proposed_head
        && c.staged_package_sha256 == staged.verified_package.package_sha256
        && c.staged_package_size == staged.verified_package.byte_length
        && c.scope_binding.signed_manifest_fingerprint
            == staged.verified_package.signed_manifest_fingerprint
        && c.proposed_head.floor == staged.verified_package.proposed_floor
        && staged.install_id == scope.install_id
        && staged.approved_plan_digest == scope.approved_plan_digest
}

#[cfg(feature = "server-root-b0v3-gate-lab")]
#[allow(dead_code, clippy::too_many_arguments)]
pub(crate) fn commit_first_provisioned_opt(
    owner: &SignedOwnerApproval,
    canonical_b0_v3: &[u8],
    scope: ExternalApprovalScope,
    staged: &StageReceipt,
    proposed: ProposedFloor,
    expected_before: WitnessCheckpoint,
    challenge: [u8; 32],
) -> Result<DurableStatus, DurableError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(DurableError::NotRoot);
    }
    // No timestamp or release pin is accepted from a peer. A root-owned
    // clock and the already registered B1a approval file are required.
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| DurableError::UntrustedRoot)?
        .as_secs();
    let pinned = open_provisioned_first_b1a_release().map_err(DurableError::FirstRelease)?;
    // B1d0 OS witness identity and durable floor are registered by OS root,
    // never supplied as a request property from the Windows SSH peer.
    let actor = OsWitnessLab::open_provisioned_first_install().map_err(DurableError::Witness)?;
    let observed = actor.current_checkpoint().map_err(DurableError::Witness)?;
    if observed != expected_before {
        return Err(DurableError::NeedsRepair);
    }
    let policy = load_provisioned_root_v3_policy().map_err(DurableError::OwnerReview)?;
    let (local_os, local_arch, systemd) =
        observed_root_host().map_err(DurableError::OwnerReview)?;
    // Rebuild the B0 owner wire from the SAME detached B1d0 signature, so a
    // client cannot present two individually signed but different scopes.
    let owner_wire = legacy_owner_wire(owner);
    if owner_wire.canonical_bytes() != owner.claim.canonical_bytes() {
        return Err(DurableError::Unenrolled);
    }
    let checked = policy
        .verify_review(
            canonical_b0_v3,
            &owner_wire,
            &owner.signature,
            &pinned,
            observed.revision,
            local_os,
            local_arch,
            systemd,
            now_unix,
        )
        .map_err(DurableError::OwnerReview)?;
    if !checked_matches_b1d0_scope(checked, owner, scope, pinned.receipt.package_sha256) {
        return Err(DurableError::Unenrolled);
    }
    let enrolled_owner = policy.trusted_owner();
    // The existing B1d0 destination lock and durable CAS recheck still
    // apply. A preflight alone never authorizes an ELF execution.
    commit_in(
        Destination::ProvisionedOpt,
        &pinned,
        &actor,
        owner,
        &enrolled_owner,
        scope,
        staged,
        proposed,
        expected_before,
        challenge,
        now_unix,
        CrashPoint::None,
    )
}

#[cfg(feature = "server-root-b0v3-gate-lab")]
fn legacy_owner_wire(owner: &SignedOwnerApproval) -> OwnerApprovalWireV1 {
    let floor = |h: crate::server_release_floor_lab::TrustedFloorHead| OwnerFloorWire {
        generation: h.generation,
        sequence: h.floor.sequence,
        version: [
            h.floor.version.major,
            h.floor.version.minor,
            h.floor.version.patch,
        ],
        manifest_fingerprint: h.floor.signed_manifest_fingerprint,
        chain_tip: h.chain_tip,
    };
    let c = &owner.claim;
    OwnerApprovalWireV1 {
        approver_id: c.approver_id.clone(),
        install_id: c.scope_binding.install_id.0,
        plan_digest: c.scope_binding.plan_digest.0,
        ssh_host_sha256: c.scope_binding.pinned_host_key_sha256,
        manifest_fingerprint: c.scope_binding.signed_manifest_fingerprint,
        previous: floor(c.previous_head),
        proposed: floor(c.proposed_head),
        staged_package_sha256: c.staged_package_sha256,
        staged_package_size: c.staged_package_size,
        expected_witness_revision: c.expected_witness_revision,
        approval_nonce: c.approval_nonce,
        issued_at_unix: c.issued_at_unix,
        expires_at_unix: c.expires_at_unix,
    }
}

#[cfg(feature = "server-root-b0v3-gate-lab")]
fn checked_matches_b1d0_scope(
    checked: CheckedRootV3Scope,
    owner: &SignedOwnerApproval,
    scope: ExternalApprovalScope,
    pinned_node_sha256: [u8; 32],
) -> bool {
    checked.install_id == scope.install_id.0
        && checked.plan_digest == scope.approved_plan_digest.0
        && checked.approval_nonce == owner.claim.approval_nonce
        && checked.node_sha256 == pinned_node_sha256
        && owner.claim.scope_binding.pinned_host_key_sha256 == scope.pinned_host_key_sha256
        && owner.claim.expected_witness_revision == scope.expected_witness_revision
}

// Keep producer and consumer signed-owner encodings identical, and fail
// closed on mismatched approval, selected host or staged Node identity.
#[cfg(all(test, feature = "server-root-b0v3-gate-lab"))]
mod root_production_gate_tests {
    use super::*;
    use crate::server_owner_approval_lab::SignedApprovalClaim;
    use crate::server_release_floor_lab::{LabPlanBinding, TrustedFloorHead};
    use crate::server_staging_lab::{ApprovedPlanDigest, InstallId};

    #[test]
    fn owner_wire_and_exact_root_admission_scope_cannot_diverge() {
        let owner = SignedOwnerApproval {
            claim: SignedApprovalClaim {
                approver_id: "operator-fixture".into(),
                scope_binding: LabPlanBinding {
                    install_id: InstallId([0x11; 16]),
                    plan_digest: ApprovedPlanDigest([0x22; 32]),
                    pinned_host_key_sha256: [0x33; 32],
                    signed_manifest_fingerprint: [0x44; 32],
                },
                previous_head: TrustedFloorHead::genesis(),
                proposed_head: TrustedFloorHead {
                    generation: 1,
                    floor: crate::server_release_lab::ReleaseFloor {
                        sequence: 1,
                        version: crate::server_release_lab::Version {
                            major: 0,
                            minor: 3,
                            patch: 13,
                        },
                        signed_manifest_fingerprint: [0x44; 32],
                    },
                    chain_tip: [0x55; 32],
                },
                staged_package_sha256: [0x66; 32],
                staged_package_size: 1024,
                expected_witness_revision: 0,
                approval_nonce: [0x77; 16],
                issued_at_unix: 100,
                expires_at_unix: 150,
            },
            signature: [0; 64],
        };
        let wire = legacy_owner_wire(&owner);
        assert_eq!(wire.canonical_bytes(), owner.claim.canonical_bytes());
        let scope = ExternalApprovalScope {
            install_id: InstallId([0x11; 16]),
            approved_plan_digest: ApprovedPlanDigest([0x22; 32]),
            pinned_host_key_sha256: [0x33; 32],
            independently_pinned_previous_head: TrustedFloorHead::genesis(),
            expected_witness_revision: 0,
        };
        let checked = CheckedRootV3Scope {
            install_id: [0x11; 16],
            plan_digest: [0x22; 32],
            node_sha256: [0x66; 32],
            bootstrap_helper_sha256: [0x88; 32],
            approval_nonce: [0x77; 16],
        };
        assert!(checked_matches_b1d0_scope(
            checked, &owner, scope, [0x66; 32]
        ));
        let verified_package = crate::server_release_lab::VerifiedPackageReceipt {
            signed_manifest_fingerprint: [0x44; 32],
            byte_length: 1024,
            package_sha256: [0x66; 32],
            proposed_floor: owner.claim.proposed_head.floor,
        };
        let stage = StageReceipt {
            install_id: scope.install_id,
            approved_plan_digest: scope.approved_plan_digest,
            verified_package,
            idempotent_replay: false,
        };
        let proposed = ProposedFloor {
            proposed_head: owner.claim.proposed_head,
            idempotent_replay: false,
            requires_external_witness: true,
        };
        let before = WitnessCheckpoint::genesis();
        assert!(frame_matches_b1d0_inputs(
            &owner, scope, &stage, proposed, before
        ));
        let mut bad_stage = stage.clone();
        bad_stage.verified_package.package_sha256[0] ^= 1;
        assert!(!frame_matches_b1d0_inputs(
            &owner, scope, &bad_stage, proposed, before
        ));
        let mut bad_scope = scope;
        bad_scope.approved_plan_digest.0[0] ^= 1;
        assert!(!frame_matches_b1d0_inputs(
            &owner, bad_scope, &stage, proposed, before
        ));
        let mut bad_before = before;
        bad_before.revision += 1;
        assert!(!frame_matches_b1d0_inputs(
            &owner, scope, &stage, proposed, bad_before
        ));
        let mut bad_proposed = proposed;
        bad_proposed.proposed_head.floor.sequence += 1;
        assert!(!frame_matches_b1d0_inputs(
            &owner,
            scope,
            &stage,
            bad_proposed,
            before
        ));
        for mutation in 0..6 {
            let mut different = checked;
            let mut different_scope = scope;
            let mut different_package = [0x66; 32];
            match mutation {
                0 => different.install_id[0] ^= 1,
                1 => different.plan_digest[0] ^= 1,
                2 => different.node_sha256[0] ^= 1,
                3 => different.approval_nonce[0] ^= 1,
                4 => different_scope.pinned_host_key_sha256[0] ^= 1,
                5 => different_package[0] ^= 1,
                _ => unreachable!(),
            }
            assert!(!checked_matches_b1d0_scope(
                different,
                &owner,
                different_scope,
                different_package,
            ));
        }
    }
}
