//! B1b1c0 witness is an INERT cooperating-writer lab, not an installer.
//! All signing keys and server identities are synthetic test fixtures only.
#![cfg(all(target_os = "linux", feature = "server-bootstrap-b1b1c-witness-lab"))]

use aquilum_core::server_owner_approval_lab::{
    ApprovalError, ExternalApprovalScope, SignedApprovalClaim,
    SignedOwnerApproval, TrustedOwnerApprover,
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
use aquilum_core::server_witness_lab::{
    InertWitnessLedgerLab, WitnessCheckpoint, WitnessDecision, WitnessError,
};
use ring::{digest, signature::{self, KeyPair}};
use std::fs;
use std::io::Cursor;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::sync::Arc;
use std::thread;
use tempfile::TempDir;

const NOW: u64 = 1_800_000_000;
const PKG1: &[u8] = b"AKULA-B1b1c0-INERT-SYNTHETIC-PACKAGE-A";
const PKG2: &[u8] = b"AKULA-B1b1c0-INERT-SYNTHETIC-PACKAGE-B";
const V1: Version = Version { major: 0, minor: 3, patch: 9 };
const V2: Version = Version { major: 0, minor: 4, patch: 0 };

fn release(payload: &[u8], sequence: u64, version: Version) -> VerifiedRelease {
    // Public, deterministic TEST signing key. NOT an Akula production key.
    let key = signature::Ed25519KeyPair::from_seed_unchecked(&[42u8; 32]).unwrap();
    let manifest = ReleaseManifestV1 {
        platform: TargetPlatform::LinuxX86_64,
        version,
        release_sequence: sequence,
        package_size: payload.len() as u64,
        package_sha256: digest::digest(&digest::SHA256, payload)
            .as_ref().try_into().unwrap(),
        not_before_unix: NOW - 30,
        expires_unix: NOW + 3600,
        signer_id: "akula-inert-test-release".into(),
    };
    let signed = SignedRelease {
        signature: key.sign(&manifest.canonical_signed_bytes())
            .as_ref().try_into().unwrap(),
        manifest,
    };
    let trusted = TrustedReleaseSigner {
        signer_id: "akula-inert-test-release".into(),
        ed25519_public: key.public_key().as_ref().try_into().unwrap(),
    };
    verify_signed_release(&signed, &trusted, ReleaseFloor::initial(), NOW).unwrap()
}

fn owner_key() -> signature::Ed25519KeyPair {
    signature::Ed25519KeyPair::from_seed_unchecked(&[43u8; 32]).unwrap()
}
fn trusted_owner() -> TrustedOwnerApprover {
    let key = owner_key();
    TrustedOwnerApprover {
        approver_id: "inert-test-owner".into(),
        public_key_ed25519: key.public_key().as_ref().try_into().unwrap(),
    }
}
fn sign(mut claim: SignedApprovalClaim, nonce: u8) -> SignedOwnerApproval {
    claim.approval_nonce = [nonce; 16];
    let signature = owner_key().sign(&claim.canonical_bytes());
    SignedOwnerApproval {
        claim,
        signature: signature.as_ref().try_into().unwrap(),
    }
}

#[derive(Clone)]
struct Case {
    signed: SignedOwnerApproval,
    scope: ExternalApprovalScope,
    staged: StageReceipt,
    proposed: ProposedFloor,
}

struct Fixture {
    _stage_dir: TempDir,
    _floor_dir: TempDir,
    witness_dir: TempDir,
    stage: PrivateStagingLab,
    floor: FloorLedgerLab,
    witness: InertWitnessLedgerLab,
}

impl Fixture {
    fn new() -> Self {
        let stage_dir = tempfile::tempdir().unwrap();
        let floor_dir = tempfile::tempdir().unwrap();
        let witness_dir = tempfile::tempdir().unwrap();
        for tmp in [&stage_dir, &floor_dir, &witness_dir] {
            fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o700)).unwrap();
        }
        let stage = PrivateStagingLab::open(stage_dir.path()).unwrap();
        let floor = FloorLedgerLab::open(floor_dir.path()).unwrap();
        let witness = InertWitnessLedgerLab::open(witness_dir.path()).unwrap();
        Self { _stage_dir: stage_dir, _floor_dir: floor_dir,
            witness_dir, stage, floor, witness }
    }

    fn prepare(
        &self,
        identity: u8,
        nonce: u8,
        payload: &[u8],
        sequence: u64,
        version: Version,
        previous: TrustedFloorHead,
    ) -> Case {
        let verified = release(payload, sequence, version);
        let install_id = InstallId([identity; 16]);
        let plan_digest = ApprovedPlanDigest([identity.wrapping_add(1); 32]);
        let staged = self.stage.stage(
            install_id, plan_digest, &verified, &mut Cursor::new(payload),
        ).unwrap();
        let binding = LabPlanBinding {
            install_id,
            plan_digest,
            pinned_host_key_sha256: [0x91; 32],
            signed_manifest_fingerprint: staged.verified_package.signed_manifest_fingerprint,
        };
        let proposed = self.floor.propose(
            previous, binding, &staged, &verified, &mut Cursor::new(payload),
        ).unwrap();
        let scope = ExternalApprovalScope {
            install_id,
            approved_plan_digest: plan_digest,
            pinned_host_key_sha256: binding.pinned_host_key_sha256,
            independently_pinned_previous_head: previous,
            expected_witness_revision: previous.generation,
        };
        let claim = SignedApprovalClaim {
            approver_id: "inert-test-owner".into(),
            scope_binding: binding,
            previous_head: previous,
            proposed_head: proposed.proposed_head,
            staged_package_sha256: staged.verified_package.package_sha256,
            staged_package_size: staged.verified_package.byte_length,
            expected_witness_revision: previous.generation,
            approval_nonce: [nonce; 16],
            issued_at_unix: NOW - 15,
            expires_at_unix: NOW + 300,
        };
        Case { signed: sign(claim, nonce), scope, staged, proposed }
    }

    fn write(&self, case: &Case, checkpoint: WitnessCheckpoint)
        -> Result<WitnessDecision, WitnessError>
    {
        self.witness.record_signed_approval(
            &case.signed, &trusted_owner(), case.scope,
            &case.staged, case.proposed, checkpoint, NOW,
        )
    }

    fn first(&self) -> Case {
        self.prepare(1, 0x44, PKG1, 17, V1, TrustedFloorHead::genesis())
    }
}

fn checkpoint(decision: WitnessDecision) -> WitnessCheckpoint {
    match decision {
        WitnessDecision::NewlyRecorded { checkpoint } |
        WitnessDecision::AlreadyRecorded { checkpoint } => checkpoint,
    }
}
fn record_path(dir: &TempDir, revision: u64, pending: bool) -> std::path::PathBuf {
    dir.path().join(format!("witness-{revision:016x}.{}",
        if pending { "pending" } else { "record" }))
}

#[test]
fn valid_owner_signature_is_reverified_and_persists_only_inert_record() {
    let f = Fixture::new();
    let first = f.first();
    let decision = f.write(&first, WitnessCheckpoint::genesis()).unwrap();
    assert!(matches!(decision, WitnessDecision::NewlyRecorded { .. }));
    assert!(decision.requires_separate_activation_authority());
    let cp = checkpoint(decision);
    assert_eq!(cp.revision, 1);
    assert_eq!(cp.accepted_floor, first.proposed.proposed_head);
    assert!(record_path(&f.witness_dir, 1, false).exists());
    assert_eq!(fs::read_dir(f.witness_dir.path()).unwrap().count(), 1);
}

#[test]
fn lost_ack_retry_after_new_handle_reopens_as_already_recorded_only() {
    let f = Fixture::new();
    let first = f.first();
    let first_decision = f.write(&first, WitnessCheckpoint::genesis()).unwrap();
    let second_handle = InertWitnessLedgerLab::open(f.witness_dir.path()).unwrap();
    let replay = second_handle.record_signed_approval(
        &first.signed, &trusted_owner(), first.scope, &first.staged,
        first.proposed, WitnessCheckpoint::genesis(), NOW,
    ).unwrap();
    assert!(matches!(replay, WitnessDecision::AlreadyRecorded { .. }));
    assert_eq!(checkpoint(replay), checkpoint(first_decision));
    assert!(replay.requires_separate_activation_authority());
    assert_eq!(fs::read_dir(f.witness_dir.path()).unwrap().count(), 1);
}

#[test]
fn public_cas_request_or_tampered_signature_cannot_bypass_inner_verification() {
    let f = Fixture::new();
    let mut case = f.first();
    case.signed.signature[0] ^= 1;
    assert!(matches!(
        f.write(&case, WitnessCheckpoint::genesis()),
        Err(WitnessError::Approval(ApprovalError::BadSignature))
    ));
    assert_eq!(fs::read_dir(f.witness_dir.path()).unwrap().count(), 0);
}

#[test]
fn swapped_approver_key_host_or_signed_plan_is_refused_before_writes() {
    let f = Fixture::new();
    let case = f.first();
    let mut wrong_key = trusted_owner();
    wrong_key.public_key_ed25519[0] ^= 1;
    assert!(matches!(
        f.witness.record_signed_approval(
            &case.signed, &wrong_key, case.scope, &case.staged,
            case.proposed, WitnessCheckpoint::genesis(), NOW,
        ),
        Err(WitnessError::Approval(ApprovalError::BadSignature))
    ));
    let mut wrong_scope = case.scope;
    wrong_scope.pinned_host_key_sha256[0] ^= 1;
    assert!(matches!(
        f.witness.record_signed_approval(
            &case.signed, &trusted_owner(), wrong_scope, &case.staged,
            case.proposed, WitnessCheckpoint::genesis(), NOW,
        ),
        Err(WitnessError::Approval(ApprovalError::ScopeMismatch))
    ));
    assert_eq!(fs::read_dir(f.witness_dir.path()).unwrap().count(), 0);
}

#[test]
fn new_nonce_on_already_used_revision_causes_compare_and_swap_conflict() {
    let f = Fixture::new();
    let first = f.first();
    f.write(&first, WitnessCheckpoint::genesis()).unwrap();
    let mut other = first.clone();
    other.signed = sign(first.signed.claim.clone(), 0x55);
    assert!(matches!(
        f.write(&other, WitnessCheckpoint::genesis()),
        Err(WitnessError::CasConflict)
    ));
    assert_eq!(fs::read_dir(f.witness_dir.path()).unwrap().count(), 1);
}

#[test]
fn future_revision_cannot_reuse_consumed_nonce_from_prior_revision() {
    let f = Fixture::new();
    let first = f.first();
    let cp = checkpoint(f.write(&first, WitnessCheckpoint::genesis()).unwrap());
    let second = f.prepare(2, 0x44, PKG2, 18, V2, cp.accepted_floor);
    assert!(matches!(
        f.write(&second, cp),
        Err(WitnessError::ConsumedNonce)
    ));
    assert!(!record_path(&f.witness_dir, 2, false).exists());
}

#[test]
fn two_monotonic_versions_write_two_independent_chained_records() {
    let f = Fixture::new();
    let first = f.first();
    let cp = checkpoint(f.write(&first, WitnessCheckpoint::genesis()).unwrap());
    let second = f.prepare(2, 0x45, PKG2, 18, V2, cp.accepted_floor);
    let decision = f.write(&second, cp).unwrap();
    assert!(matches!(decision, WitnessDecision::NewlyRecorded { .. }));
    let next = checkpoint(decision);
    assert_eq!(next.revision, 2);
    assert_eq!(next.accepted_floor.floor.sequence, 18);
    assert_ne!(next.record_tip, cp.record_tip);
    assert_eq!(fs::read_dir(f.witness_dir.path()).unwrap().count(), 2);
}

#[test]
fn corrupted_record_blocks_progress_even_with_correct_owner_signature() {
    let f = Fixture::new();
    let first = f.first();
    let cp = checkpoint(f.write(&first, WitnessCheckpoint::genesis()).unwrap());
    let p = record_path(&f.witness_dir, 1, false);
    let mut raw = fs::read(&p).unwrap();
    raw[40] ^= 1;
    fs::write(&p, raw).unwrap();
    let second = f.prepare(2, 0x45, PKG2, 18, V2, cp.accepted_floor);
    assert!(matches!(
        f.write(&second, cp),
        Err(WitnessError::CorruptRecord)
    ));
    assert!(!record_path(&f.witness_dir, 2, false).exists());
}

#[test]
fn pending_record_requires_explicit_repair_and_is_never_deleted() {
    let f = Fixture::new();
    let first = f.first();
    let pending = record_path(&f.witness_dir, 1, true);
    fs::write(&pending, b"aborted before fsync or rename").unwrap();
    assert!(matches!(
        f.write(&first, WitnessCheckpoint::genesis()),
        Err(WitnessError::NeedsRepair)
    ));
    assert!(pending.exists());
    assert!(!record_path(&f.witness_dir, 1, false).exists());
}

#[test]
fn separate_handles_serialize_without_duplicate_record_or_grant() {
    let f = Fixture::new();
    let first = Arc::new(f.first());
    let path = Arc::new(f.witness_dir.path().to_path_buf());
    let handles: Vec<_> = (0..2).map(|_| {
        let copy = Arc::clone(&first);
        let path = Arc::clone(&path);
        thread::spawn(move || {
            let witness = InertWitnessLedgerLab::open(&path).unwrap();
            witness.record_signed_approval(
                &copy.signed, &trusted_owner(), copy.scope, &copy.staged,
                copy.proposed, WitnessCheckpoint::genesis(), NOW,
            ).unwrap()
        })
    }).collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|d|
        matches!(d, WitnessDecision::NewlyRecorded { .. })).count(), 1);
    assert_eq!(results.iter().filter(|d|
        matches!(d, WitnessDecision::AlreadyRecorded { .. })).count(), 1);
    assert_eq!(fs::read_dir(f.witness_dir.path()).unwrap().count(), 1);
}

#[test]
fn symlink_entry_and_world_readable_witness_dir_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        InertWitnessLedgerLab::open(tmp.path()),
        Err(WitnessError::UnsafeDirectory)
    ));
    let f = Fixture::new();
    let first = f.first();
    let p = record_path(&f.witness_dir, 1, false);
    symlink("/etc/passwd", &p).unwrap();
    assert!(f.write(&first, WitnessCheckpoint::genesis()).is_err());
    assert!(p.symlink_metadata().unwrap().file_type().is_symlink());
}

#[test]
fn fabricated_external_checkpoint_and_wrong_generation_fail_closed() {
    let f = Fixture::new();
    let first = f.first();
    let mut cp = WitnessCheckpoint::genesis();
    cp.record_tip[0] = 1;
    assert!(matches!(
        f.write(&first, cp),
        Err(WitnessError::UntrustedCheckpoint)
    ));
    let accepted = checkpoint(f.write(&first, WitnessCheckpoint::genesis()).unwrap());
    let second = f.prepare(2, 0x45, PKG2, 18, V2, accepted.accepted_floor);
    assert!(matches!(
        f.write(&second, WitnessCheckpoint::genesis()),
        Err(WitnessError::UntrustedCheckpoint)
    ));
}
