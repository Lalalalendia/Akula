//! Internal B1-E2E-09 adapter: proven signed IPC review -> B1d0 typed inputs.
//! This module does NOT consume the nonce, call the witness, write /opt,
//! provide a capability to an AF_UNIX peer, or activate any service.
use super::*;
use crate::server_owner_approval_lab::{
    ExternalApprovalScope, SignedOwnerApproval,
};
use crate::server_release_floor_lab::ProposedFloor;
use crate::server_release_lab::VerifiedPackageReceipt;
use crate::server_staging_lab::StageReceipt;

#[allow(dead_code)]
#[derive(Debug)]
pub(super) struct CheckedB1d0Inputs {
    pub scope: ExternalApprovalScope,
    pub staged: StageReceipt,
    pub proposed: ProposedFloor,
    pub expected_before: WitnessCheckpoint,
}

/// The inputs to this adapter have been independently admitted:
/// owner is the verified signed owner frame, checked is verified root-side
/// plan/host/helper/release scope, receipt is the root-pinned signed B1a FD,
/// witness is the root-owned journal checkpoint. Genesis only: first install.
/// The B1d0 writer MUST re-open and recheck these authorities under its lock.
pub(super) fn bind_verified_b1d0_inputs(
    checked: crate::server_root_v3_guard::CheckedRootV3Scope,
    owner: &SignedOwnerApproval,
    receipt: VerifiedPackageReceipt,
    witness: WitnessCheckpoint,
) -> Result<CheckedB1d0Inputs, OwnerIpcError> {
    let claim = &owner.claim;
    let binding = claim.scope_binding;
    if witness != WitnessCheckpoint::genesis()
        || claim.previous_head != witness.accepted_floor
        || claim.expected_witness_revision != witness.revision
        || claim.proposed_head.generation != 1
        || claim.proposed_head.chain_tip == [0; 32]
        || claim.proposed_head.floor != receipt.proposed_floor
        || claim.proposed_head.floor.sequence == 0
        || checked.install_id != binding.install_id.0
        || checked.plan_digest != binding.plan_digest.0
        || checked.node_sha256 != receipt.package_sha256
        || checked.bootstrap_helper_sha256 == [0; 32]
        || checked.approval_nonce != claim.approval_nonce
        || claim.approval_nonce == [0; 16]
        || claim.staged_package_sha256 != receipt.package_sha256
        || claim.staged_package_size != receipt.byte_length
        || binding.signed_manifest_fingerprint != receipt.signed_manifest_fingerprint
        || binding.pinned_host_key_sha256 == [0; 32]
        || receipt.package_sha256 == [0; 32]
        || receipt.signed_manifest_fingerprint == [0; 32]
        || receipt.byte_length == 0
    {
        return Err(OwnerIpcError::InvalidFrame);
    }
    Ok(CheckedB1d0Inputs {
        scope: ExternalApprovalScope {
            install_id: binding.install_id,
            approved_plan_digest: binding.plan_digest,
            pinned_host_key_sha256: binding.pinned_host_key_sha256,
            independently_pinned_previous_head: witness.accepted_floor,
            expected_witness_revision: witness.revision,
        },
        staged: StageReceipt {
            install_id: binding.install_id,
            approved_plan_digest: binding.plan_digest,
            verified_package: receipt,
            idempotent_replay: false,
        },
        proposed: ProposedFloor {
            proposed_head: claim.proposed_head,
            idempotent_replay: false,
            requires_external_witness: true,
        },
        expected_before: witness,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_owner_approval_lab::SignedApprovalClaim;
    use crate::server_release_floor_lab::{LabPlanBinding, TrustedFloorHead};
    use crate::server_release_lab::{ReleaseFloor, Version};
    use crate::server_root_v3_guard::CheckedRootV3Scope;
    use crate::server_staging_lab::{ApprovedPlanDigest, InstallId};

    fn fixture() -> (CheckedRootV3Scope, SignedOwnerApproval, VerifiedPackageReceipt) {
        let manifest = [0x44; 32];
        let floor = ReleaseFloor {
            sequence: 1,
            version: Version { major: 0, minor: 3, patch: 14 },
            signed_manifest_fingerprint: manifest,
        };
        let owner = SignedOwnerApproval {
            claim: SignedApprovalClaim {
                approver_id: "owner-fixture".into(),
                scope_binding: LabPlanBinding {
                    install_id: InstallId([0x11; 16]),
                    plan_digest: ApprovedPlanDigest([0x22; 32]),
                    pinned_host_key_sha256: [0x33; 32],
                    signed_manifest_fingerprint: manifest,
                },
                previous_head: TrustedFloorHead::genesis(),
                proposed_head: TrustedFloorHead {
                    generation: 1,
                    floor,
                    chain_tip: [0x55; 32],
                },
                staged_package_sha256: [0x66; 32],
                staged_package_size: 1024,
                expected_witness_revision: 0,
                approval_nonce: [0x77; 16],
                issued_at_unix: 100,
                expires_at_unix: 150,
            },
            signature: [0x88; 64],
        };
        let checked = CheckedRootV3Scope {
            install_id: [0x11; 16],
            plan_digest: [0x22; 32],
            node_sha256: [0x66; 32],
            bootstrap_helper_sha256: [0x99; 32],
            approval_nonce: [0x77; 16],
        };
        let receipt = VerifiedPackageReceipt {
            signed_manifest_fingerprint: manifest,
            byte_length: 1024,
            package_sha256: [0x66; 32],
            proposed_floor: floor,
        };
        (checked, owner, receipt)
    }

    #[test]
    fn verified_first_install_claim_is_mapped_to_existing_cas_types() {
        let (checked, owner, receipt) = fixture();
        let result = bind_verified_b1d0_inputs(
            checked, &owner, receipt.clone(), WitnessCheckpoint::genesis()
        ).unwrap();
        assert_eq!(result.scope.install_id.0, checked.install_id);
        assert_eq!(result.scope.approved_plan_digest.0, checked.plan_digest);
        assert_eq!(result.scope.pinned_host_key_sha256, [0x33; 32]);
        assert_eq!(result.staged.verified_package, receipt);
        assert_eq!(result.staged.install_id, result.scope.install_id);
        assert_eq!(result.expected_before, WitnessCheckpoint::genesis());
        assert_eq!(result.proposed.proposed_head, owner.claim.proposed_head);
        assert!(!result.staged.idempotent_replay);
        assert!(!result.proposed.idempotent_replay);
        assert!(result.proposed.requires_external_witness);
    }

    #[test]
    fn mismatched_signed_owner_or_independent_fd_refuses_before_cas() {
        for case in 0..17 {
            let (mut checked, mut owner, mut receipt) = fixture();
            let mut witness = WitnessCheckpoint::genesis();
            match case {
                0 => checked.install_id[0] ^= 1,
                1 => checked.plan_digest[0] ^= 1,
                2 => checked.node_sha256[0] ^= 1,
                3 => checked.bootstrap_helper_sha256 = [0; 32],
                4 => checked.approval_nonce[0] ^= 1,
                5 => owner.claim.scope_binding.pinned_host_key_sha256 = [0; 32],
                6 => owner.claim.staged_package_sha256[0] ^= 1,
                7 => owner.claim.staged_package_size += 1,
                8 => owner.claim.scope_binding.signed_manifest_fingerprint[0] ^= 1,
                9 => owner.claim.proposed_head.floor.sequence += 1,
                10 => owner.claim.proposed_head.generation = 0,
                11 => owner.claim.proposed_head.chain_tip = [0; 32],
                12 => owner.claim.approval_nonce = [0; 16],
                13 => owner.claim.expected_witness_revision += 1,
                14 => owner.claim.previous_head.generation += 1,
                15 => receipt.package_sha256[0] ^= 1,
                16 => witness.revision += 1,
                _ => unreachable!(),
            }
            assert!(
                bind_verified_b1d0_inputs(checked, &owner, receipt, witness).is_err(),
                "case {case}: mismatched B1d0 input was accepted"
            );
        }
    }

    #[test]
    fn malformed_or_missing_signed_release_metadata_refuses_before_cas() {
        let (checked, owner, receipt) = fixture();
        for case in 0..5 {
            let mut bad = receipt.clone();
            match case {
                0 => bad.signed_manifest_fingerprint = [0; 32],
                1 => bad.package_sha256 = [0; 32],
                2 => bad.byte_length = 0,
                3 => bad.proposed_floor.signed_manifest_fingerprint[0] ^= 1,
                4 => bad.proposed_floor.sequence = 0,
                _ => unreachable!(),
            }
            assert!(
                bind_verified_b1d0_inputs(
                    checked, &owner, bad, WitnessCheckpoint::genesis()
                ).is_err(),
                "case {case}: malformed verified FD receipt was accepted"
            );
        }
    }
}
