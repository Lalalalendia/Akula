//! Synthetic signed owner approval -> inert witness CAS intent.
//! No private production keys, no SSH, systemd, privilege or activation.
#![cfg(all(target_os = "linux", feature = "server-bootstrap-b1b1b-approval-lab"))]

use aquilum_core::server_owner_approval_lab::{
    verify_owner_approval, ApprovalError, ExternalApprovalScope, SignedApprovalClaim,
    SignedOwnerApproval, TrustedOwnerApprover, MAX_APPROVAL_VALIDITY_SECONDS,
};
use aquilum_core::server_release_floor_lab::{
    FloorLedgerLab, LabPlanBinding, ProposedFloor, TrustedFloorHead,
};
use aquilum_core::server_release_lab::{
    verify_signed_release, ReleaseFloor, ReleaseManifestV1, SignedRelease,
    TargetPlatform, TrustedReleaseSigner, VerifiedRelease, Version,
};
use aquilum_core::server_staging_lab::{
    ApprovedPlanDigest, InstallId, PrivateStagingLab, StageReceipt,
};
use ring::{digest, signature::{self, KeyPair}};
use std::{fs, io::Cursor, os::unix::fs::PermissionsExt};
use tempfile::TempDir;

const NOW: u64 = 1_800_000_000;
const TEST_PACKAGE: &[u8] = b"Akula B1b1b synthetic, inert signed test package";

fn verify_test_release() -> VerifiedRelease {
    let key = signature::Ed25519KeyPair::from_seed_unchecked(&[42u8; 32]).unwrap();
    let manifest = ReleaseManifestV1 {
        platform: TargetPlatform::LinuxX86_64,
        version: Version { major: 0, minor: 3, patch: 9 },
        release_sequence: 17,
        package_size: TEST_PACKAGE.len() as u64,
        package_sha256: digest::digest(&digest::SHA256, TEST_PACKAGE)
            .as_ref().try_into().unwrap(),
        not_before_unix: NOW - 60,
        expires_unix: NOW + 3600,
        signer_id: "akula-release-test".into(),
    };
    let signed = SignedRelease {
        signature: key.sign(&manifest.canonical_signed_bytes())
            .as_ref().try_into().unwrap(),
        manifest,
    };
    let trusted = TrustedReleaseSigner {
        signer_id: "akula-release-test".into(),
        ed25519_public: key.public_key().as_ref().try_into().unwrap(),
    };
    verify_signed_release(&signed, &trusted, ReleaseFloor::initial(), NOW).unwrap()
}

fn owner_key() -> signature::Ed25519KeyPair {
    // Public, deterministic test key. NOT a real owner or release key.
    signature::Ed25519KeyPair::from_seed_unchecked(&[43u8; 32]).unwrap()
}

fn owner_anchor() -> TrustedOwnerApprover {
    let key = owner_key();
    TrustedOwnerApprover {
        approver_id: "akula-test-operator".into(),
        public_key_ed25519: key.public_key().as_ref().try_into().unwrap(),
    }
}

fn sign_claim(claim: SignedApprovalClaim) -> SignedOwnerApproval {
    let signature = owner_key().sign(&claim.canonical_bytes());
    SignedOwnerApproval {
        claim, signature: signature.as_ref().try_into().unwrap(),
    }
}

struct Fixture {
    _stage_dir: TempDir,
    ledger_dir: TempDir,
    _stage: PrivateStagingLab,
    _ledger: FloorLedgerLab,
    staged: StageReceipt,
    proposed: ProposedFloor,
    scope: ExternalApprovalScope,
}

impl Fixture {
    fn new() -> Self {
        let stage_dir = tempfile::tempdir().unwrap();
        let ledger_dir = tempfile::tempdir().unwrap();
        fs::set_permissions(stage_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(ledger_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let stage = PrivateStagingLab::open(stage_dir.path()).unwrap();
        let ledger = FloorLedgerLab::open(ledger_dir.path()).unwrap();
        let verified = verify_test_release();
        let install_id = InstallId([0x11; 16]);
        let plan_digest = ApprovedPlanDigest([0x22; 32]);
        let staged = stage.stage(
            install_id, plan_digest, &verified, &mut Cursor::new(TEST_PACKAGE),
        ).unwrap();
        let binding = LabPlanBinding {
            install_id, plan_digest,
            pinned_host_key_sha256: [0x33; 32],
            signed_manifest_fingerprint: staged.verified_package.signed_manifest_fingerprint,
        };
        let previous = TrustedFloorHead::genesis();
        let proposed = ledger.propose(
            previous, binding, &staged, &verified, &mut Cursor::new(TEST_PACKAGE),
        ).unwrap();
        let scope = ExternalApprovalScope {
            install_id,
            approved_plan_digest: plan_digest,
            pinned_host_key_sha256: binding.pinned_host_key_sha256,
            independently_pinned_previous_head: previous,
            expected_witness_revision: 0,
        };
        Self { _stage_dir: stage_dir, ledger_dir, _stage: stage, _ledger: ledger, staged, proposed, scope }
    }

    fn claim(&self) -> SignedApprovalClaim {
        SignedApprovalClaim {
            approver_id: "akula-test-operator".into(),
            scope_binding: LabPlanBinding {
                install_id: self.scope.install_id,
                plan_digest: self.scope.approved_plan_digest,
                pinned_host_key_sha256: self.scope.pinned_host_key_sha256,
                signed_manifest_fingerprint: self.staged.verified_package.signed_manifest_fingerprint,
            },
            previous_head: self.scope.independently_pinned_previous_head,
            proposed_head: self.proposed.proposed_head,
            staged_package_sha256: self.staged.verified_package.package_sha256,
            staged_package_size: self.staged.verified_package.byte_length,
            expected_witness_revision: self.scope.expected_witness_revision,
            approval_nonce: [0x44; 16],
            issued_at_unix: NOW - 30,
            expires_at_unix: NOW + 300,
        }
    }

    fn check(&self, signed: &SignedOwnerApproval) -> Result<aquilum_core::server_owner_approval_lab::InertWitnessCasRequest, ApprovalError> {
        verify_owner_approval(
            signed, &owner_anchor(), self.scope, &self.staged, self.proposed, NOW,
        )
    }
}

#[test]
fn detached_signed_claim_binds_all_fields_and_returns_only_inert_cas_request() {
    let fixture = Fixture::new();
    let signed = sign_claim(fixture.claim());
    let cas = fixture.check(&signed).unwrap();
    assert_eq!((cas.expected_revision, cas.desired_revision), (0, 1));
    assert_eq!(cas.previous_head, TrustedFloorHead::genesis());
    assert_eq!(cas.proposed_head, fixture.proposed.proposed_head);
    assert_eq!(cas.install_id, fixture.scope.install_id);
    assert!(cas.requires_external_commit);
    assert_eq!(fs::read_dir(fixture.ledger_dir.path()).unwrap().count(), 1);
}

#[test]
fn changed_signed_host_plan_release_or_nonce_without_resigning_is_refused() {
    let f = Fixture::new();
    let original = sign_claim(f.claim());
    let mut changed = original.clone();
    changed.claim.scope_binding.pinned_host_key_sha256[0] ^= 1;
    assert_eq!(f.check(&changed).unwrap_err(), ApprovalError::BadSignature);
    let mut changed = original.clone();
    changed.claim.scope_binding.plan_digest.0[0] ^= 1;
    assert_eq!(f.check(&changed).unwrap_err(), ApprovalError::BadSignature);
    let mut changed = original.clone();
    changed.claim.staged_package_sha256[0] ^= 1;
    assert_eq!(f.check(&changed).unwrap_err(), ApprovalError::BadSignature);
    let mut changed = original;
    changed.claim.approval_nonce[0] ^= 1;
    assert_eq!(f.check(&changed).unwrap_err(), ApprovalError::BadSignature);
}

#[test]
fn wrong_pinned_approver_key_or_id_is_never_accepted() {
    let f = Fixture::new();
    let signed = sign_claim(f.claim());
    let mut wrong = owner_anchor();
    wrong.public_key_ed25519 = [0x55; 32];
    assert_eq!(
        verify_owner_approval(&signed, &wrong, f.scope, &f.staged, f.proposed, NOW).unwrap_err(),
        ApprovalError::BadSignature
    );
    let mut wrong = owner_anchor();
    wrong.approver_id.push('x');
    assert_eq!(
        verify_owner_approval(&signed, &wrong, f.scope, &f.staged, f.proposed, NOW).unwrap_err(),
        ApprovalError::UntrustedApprover
    );
}

#[test]
fn properly_signed_claim_for_different_host_or_plan_cannot_be_repurposed() {
    let f = Fixture::new();
    let mut claim = f.claim();
    claim.scope_binding.pinned_host_key_sha256[0] ^= 1;
    assert_eq!(f.check(&sign_claim(claim)).unwrap_err(), ApprovalError::ScopeMismatch);
    let mut claim = f.claim();
    claim.scope_binding.plan_digest.0[0] ^= 1;
    assert_eq!(f.check(&sign_claim(claim)).unwrap_err(), ApprovalError::ScopeMismatch);
    let mut claim = f.claim();
    claim.scope_binding.install_id.0[0] ^= 1;
    assert_eq!(f.check(&sign_claim(claim)).unwrap_err(), ApprovalError::ScopeMismatch);
}

#[test]
fn expired_future_and_overlong_approvals_are_refused() {
    let f = Fixture::new();
    let mut claim = f.claim();
    claim.issued_at_unix = NOW - 800;
    claim.expires_at_unix = NOW - 1;
    assert_eq!(f.check(&sign_claim(claim)).unwrap_err(), ApprovalError::Expired);
    let mut claim = f.claim();
    claim.issued_at_unix = NOW + 1;
    claim.expires_at_unix = NOW + 300;
    assert_eq!(f.check(&sign_claim(claim)).unwrap_err(), ApprovalError::NotYetValid);
    let mut claim = f.claim();
    claim.issued_at_unix = NOW - 1;
    claim.expires_at_unix = NOW + MAX_APPROVAL_VALIDITY_SECONDS + 20;
    assert_eq!(f.check(&sign_claim(claim)).unwrap_err(), ApprovalError::MalformedClaim);
}

#[test]
fn stale_witness_revision_and_wrong_independent_floor_refuse() {
    let f = Fixture::new();
    let mut claim = f.claim();
    claim.expected_witness_revision = 1;
    assert_eq!(
        f.check(&sign_claim(claim)).unwrap_err(), ApprovalError::WitnessRevisionMismatch
    );
    let mut scope = f.scope;
    scope.expected_witness_revision = 1;
    let signed = sign_claim(f.claim());
    assert_eq!(
        verify_owner_approval(&signed, &owner_anchor(), scope, &f.staged, f.proposed, NOW).unwrap_err(),
        ApprovalError::WitnessRevisionMismatch
    );
    let mut scope = f.scope;
    scope.independently_pinned_previous_head.chain_tip[0] ^= 1;
    assert_eq!(
        verify_owner_approval(&signed, &owner_anchor(), scope, &f.staged, f.proposed, NOW).unwrap_err(),
        ApprovalError::ScopeMismatch
    );
}

#[test]
fn mismatching_staged_receipt_or_proposal_is_denied() {
    let f = Fixture::new();
    let signed = sign_claim(f.claim());
    let mut stage = f.staged.clone();
    stage.verified_package.package_sha256[0] ^= 1;
    assert_eq!(
        verify_owner_approval(&signed, &owner_anchor(), f.scope, &stage, f.proposed, NOW).unwrap_err(),
        ApprovalError::StageMismatch
    );
    let mut proposed = f.proposed;
    proposed.requires_external_witness = false;
    assert_eq!(
        verify_owner_approval(&signed, &owner_anchor(), f.scope, &f.staged, proposed, NOW).unwrap_err(),
        ApprovalError::ProposalMismatch
    );
}

#[test]
fn malformed_fields_and_empty_nonce_are_refused_even_with_valid_signature() {
    let f = Fixture::new();
    let mut claim = f.claim();
    claim.approval_nonce = [0; 16];
    assert_eq!(f.check(&sign_claim(claim)).unwrap_err(), ApprovalError::MalformedClaim);
    let mut claim = f.claim();
    claim.approver_id = "../override".into();
    assert_eq!(f.check(&sign_claim(claim)).unwrap_err(), ApprovalError::MalformedClaim);
    let mut claim = f.claim();
    claim.staged_package_size = 0;
    assert_eq!(f.check(&sign_claim(claim)).unwrap_err(), ApprovalError::MalformedClaim);
}

#[test]
fn mismatched_proposed_floor_or_transition_cannot_be_committed() {
    let f = Fixture::new();
    let mut claim = f.claim();
    claim.proposed_head.floor.sequence += 1;
    assert_eq!(f.check(&sign_claim(claim)).unwrap_err(), ApprovalError::StageMismatch);
    let mut proposed = f.proposed;
    proposed.proposed_head.generation = 2;
    let mut claim = f.claim();
    claim.proposed_head = proposed.proposed_head;
    assert_eq!(
        verify_owner_approval(&sign_claim(claim), &owner_anchor(),
            f.scope, &f.staged, proposed, NOW).unwrap_err(),
        ApprovalError::InvalidTransition
    );
}

#[test]
fn linux_b1b1b_signature_bytes_stay_identical_to_original_contract() {
    // Independent historical encoder, test only: catches silent changes
    // to owner approval signatures when a Windows signer is introduced.
    let claim = Fixture::new().claim();
    let mut historical = Vec::new();
    historical.extend_from_slice(b"Akula B1b1b owner approval v1: inert witness CAS\0");
    historical.push(1);
    historical.push(claim.approver_id.len() as u8);
    historical.extend_from_slice(claim.approver_id.as_bytes());
    historical.extend_from_slice(&claim.scope_binding.install_id.0);
    historical.extend_from_slice(&claim.scope_binding.plan_digest.0);
    historical.extend_from_slice(&claim.scope_binding.pinned_host_key_sha256);
    historical.extend_from_slice(&claim.scope_binding.signed_manifest_fingerprint);
    for head in [claim.previous_head, claim.proposed_head] {
        historical.extend_from_slice(&head.generation.to_be_bytes());
        historical.extend_from_slice(&head.floor.sequence.to_be_bytes());
        for version in [head.floor.version.major, head.floor.version.minor,
            head.floor.version.patch]
        {
            historical.extend_from_slice(&version.to_be_bytes());
        }
        historical.extend_from_slice(&head.floor.signed_manifest_fingerprint);
        historical.extend_from_slice(&head.chain_tip);
    }
    historical.extend_from_slice(&claim.staged_package_sha256);
    historical.extend_from_slice(&claim.staged_package_size.to_be_bytes());
    historical.extend_from_slice(&claim.expected_witness_revision.to_be_bytes());
    historical.extend_from_slice(&claim.approval_nonce);
    historical.extend_from_slice(&claim.issued_at_unix.to_be_bytes());
    historical.extend_from_slice(&claim.expires_at_unix.to_be_bytes());
    assert_eq!(claim.canonical_bytes(), historical);
}
