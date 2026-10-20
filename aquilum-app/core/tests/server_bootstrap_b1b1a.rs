//! B1b1a: no installation, no SSH, no elevation, no activation.
//! Every byte and identity in this lab is synthetic and disposable.
#![cfg(all(target_os = "linux", feature = "server-bootstrap-b1b1-floor-lab"))]

use aquilum_core::server_release_floor_lab::{
    FloorLedgerLab, LabPlanBinding, LedgerError, TrustedFloorHead,
};
use aquilum_core::server_release_lab::{
    verify_signed_release, ReleaseFloor, ReleaseManifestV1, SignedRelease,
    TargetPlatform, TrustedReleaseSigner, VerifiedRelease, Version,
};
use aquilum_core::server_staging_lab::{
    ApprovedPlanDigest, InstallId, PrivateStagingLab, StageReceipt,
};
use ring::{digest, signature::{self, KeyPair}};
use std::fs;
use std::io::Cursor;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::sync::Arc;
use std::thread;
use tempfile::TempDir;

const NOW: u64 = 1_800_000_000;
const PACKAGE_A: &[u8] = b"AKULA-B1b1a-TEST-ONLY-payload-a";
const PACKAGE_B: &[u8] = b"AKULA-B1b1a-TEST-ONLY-payload-b";

fn signed_release(payload: &[u8], seq: u64, version: Version) -> VerifiedRelease {
    let signing_key = signature::Ed25519KeyPair::from_seed_unchecked(&[42u8; 32]).unwrap();
    let manifest = ReleaseManifestV1 {
        platform: TargetPlatform::LinuxX86_64,
        version,
        release_sequence: seq,
        package_size: payload.len() as u64,
        package_sha256: digest::digest(&digest::SHA256, payload)
            .as_ref().try_into().unwrap(),
        not_before_unix: NOW - 60,
        expires_unix: NOW + 3600,
        signer_id: "akula-node-lab-key".into(),
    };
    let signature = signing_key.sign(&manifest.canonical_signed_bytes());
    let trust = TrustedReleaseSigner {
        signer_id: "akula-node-lab-key".into(),
        ed25519_public: signing_key.public_key().as_ref().try_into().unwrap(),
    };
    let signed = SignedRelease {
        manifest,
        signature: signature.as_ref().try_into().unwrap(),
    };
    // Only the test key is used here. The ledger itself enforces the
    // independently pinned floor from the caller, not this test verifier.
    verify_signed_release(&signed, &trust, ReleaseFloor::initial(), NOW).unwrap()
}

const V1: Version = Version { major: 0, minor: 3, patch: 9 };
const V2: Version = Version { major: 0, minor: 4, patch: 0 };

struct Fixture {
    _staging_dir: TempDir,
    ledger_dir: TempDir,
    staging: PrivateStagingLab,
    ledger: FloorLedgerLab,
}

impl Fixture {
    fn new() -> Self {
        let staging_dir = tempfile::tempdir().unwrap();
        let ledger_dir = tempfile::tempdir().unwrap();
        fs::set_permissions(staging_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(ledger_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let staging = PrivateStagingLab::open(staging_dir.path()).unwrap();
        let ledger = FloorLedgerLab::open(ledger_dir.path()).unwrap();
        Self { _staging_dir: staging_dir, ledger_dir, staging, ledger }
    }

    fn prepare(
        &self,
        id: u8,
        sequence: u64,
        version: Version,
        payload: &[u8],
    ) -> (VerifiedRelease, StageReceipt, LabPlanBinding) {
        let verified = signed_release(payload, sequence, version);
        let install_id = InstallId([id; 16]);
        let plan_digest = ApprovedPlanDigest([id.wrapping_add(1); 32]);
        let staged = self.staging.stage(
            install_id, plan_digest, &verified, &mut Cursor::new(payload)
        ).unwrap();
        let binding = LabPlanBinding {
            install_id,
            plan_digest,
            pinned_host_key_sha256: [0x91; 32], // synthetic host fingerprint
            signed_manifest_fingerprint: staged.verified_package.signed_manifest_fingerprint,
        };
        (verified, staged, binding)
    }
}

fn record_path(dir: &TempDir, generation: u64, pending: bool) -> std::path::PathBuf {
    dir.path().join(format!("floor-{generation:016x}.{}",
        if pending { "pending" } else { "record" }))
}

#[test]
fn signed_bytes_are_journaled_but_never_activated_and_replay_survives_reopen() {
    let f = Fixture::new();
    let (v, staged, binding) = f.prepare(1, 17, V1, PACKAGE_A);
    let first = f.ledger.propose(
        TrustedFloorHead::genesis(), binding, &staged, &v,
        &mut Cursor::new(PACKAGE_A),
    ).unwrap();
    assert_eq!(first.proposed_head.generation, 1);
    assert_eq!(first.proposed_head.floor.sequence, 17);
    assert!(first.requires_external_witness);
    assert!(!first.idempotent_replay);
    assert!(record_path(&f.ledger_dir, 1, false).exists());
    assert!(!record_path(&f.ledger_dir, 1, true).exists());

    let second_ledger = FloorLedgerLab::open(f.ledger_dir.path()).unwrap();
    let retry = second_ledger.propose(
        TrustedFloorHead::genesis(), binding, &staged, &v,
        &mut Cursor::new(PACKAGE_A),
    ).unwrap();
    assert!(retry.idempotent_replay);
    assert!(retry.requires_external_witness);
    assert_eq!(retry.proposed_head, first.proposed_head);
    // Only a journal exists here; no process or service has been executed.
    assert_eq!(fs::read_dir(f.ledger_dir.path()).unwrap().count(), 1);
}

#[test]
fn monotonic_generation_two_uses_independently_witnessed_previous_head() {
    let f = Fixture::new();
    let (v1, s1, b1) = f.prepare(1, 17, V1, PACKAGE_A);
    let first = f.ledger.propose(
        TrustedFloorHead::genesis(), b1, &s1, &v1,
        &mut Cursor::new(PACKAGE_A),
    ).unwrap();
    // The *test harness* simulates independent witness approval by passing
    // the previous proposal explicitly. No witness is actually implemented.
    let (v2, s2, b2) = f.prepare(2, 18, V2, PACKAGE_B);
    let second = f.ledger.propose(
        first.proposed_head, b2, &s2, &v2,
        &mut Cursor::new(PACKAGE_B),
    ).unwrap();
    assert_eq!(second.proposed_head.generation, 2);
    assert_eq!(second.proposed_head.floor.sequence, 18);
    assert_ne!(second.proposed_head.chain_tip, first.proposed_head.chain_tip);
    assert!(second.requires_external_witness);
    assert_eq!(fs::read_dir(f.ledger_dir.path()).unwrap().count(), 2);
}

#[test]
fn rollback_sequence_and_semantic_version_refuse_without_mutations() {
    let f = Fixture::new();
    let (v1, s1, b1) = f.prepare(1, 17, V1, PACKAGE_A);
    let first = f.ledger.propose(
        TrustedFloorHead::genesis(), b1, &s1, &v1,
        &mut Cursor::new(PACKAGE_A),
    ).unwrap();
    let (older, stage, binding) = f.prepare(2, 16, V1, PACKAGE_B);
    let err = f.ledger.propose(
        first.proposed_head, binding, &stage, &older,
        &mut Cursor::new(PACKAGE_B),
    ).unwrap_err();
    assert!(matches!(err, LedgerError::RollbackRefused));

    let (new_seq_old_version, stage2, binding2) =
        f.prepare(3, 18, Version { major: 0, minor: 3, patch: 8 }, PACKAGE_B);
    let err = f.ledger.propose(
        first.proposed_head, binding2, &stage2, &new_seq_old_version,
        &mut Cursor::new(PACKAGE_B),
    ).unwrap_err();
    assert!(matches!(err, LedgerError::RollbackRefused));
    assert!(!record_path(&f.ledger_dir, 2, false).exists());
}

#[test]
fn digest_host_and_install_id_bindings_cannot_be_silently_swapped() {
    let f = Fixture::new();
    let (v, staged, binding) = f.prepare(1, 17, V1, PACKAGE_A);
    let mut wrong_id = binding;
    wrong_id.install_id = InstallId([3; 16]);
    assert!(matches!(
        f.ledger.propose(TrustedFloorHead::genesis(), wrong_id, &staged, &v,
            &mut Cursor::new(PACKAGE_A)),
        Err(LedgerError::InvalidBinding)
    ));
    let first = f.ledger.propose(
        TrustedFloorHead::genesis(), binding, &staged, &v,
        &mut Cursor::new(PACKAGE_A),
    ).unwrap();
    let mut wrong_host = binding;
    wrong_host.pinned_host_key_sha256[0] ^= 1;
    assert!(matches!(
        f.ledger.propose(TrustedFloorHead::genesis(), wrong_host, &staged, &v,
            &mut Cursor::new(PACKAGE_A)),
        Err(LedgerError::AnchorConflict)
    ));
    assert_eq!(first.proposed_head.generation, 1);
}

#[test]
fn checksum_corruption_and_foreign_pinned_head_refuse_open_or_write() {
    let f = Fixture::new();
    let (v, staged, binding) = f.prepare(1, 17, V1, PACKAGE_A);
    let first = f.ledger.propose(
        TrustedFloorHead::genesis(), binding, &staged, &v,
        &mut Cursor::new(PACKAGE_A),
    ).unwrap();
    let mut forged_head = first.proposed_head;
    forged_head.chain_tip[0] ^= 1;
    let (v2, s2, b2) = f.prepare(2, 18, V2, PACKAGE_B);
    assert!(matches!(
        f.ledger.propose(forged_head, b2, &s2, &v2,
            &mut Cursor::new(PACKAGE_B)),
        Err(LedgerError::AnchorConflict)
    ));

    let p = record_path(&f.ledger_dir, 1, false);
    let mut bytes = fs::read(&p).unwrap();
    bytes[60] ^= 1;
    fs::write(&p, bytes).unwrap();
    assert!(matches!(
        f.ledger.propose(first.proposed_head, b2, &s2, &v2,
            &mut Cursor::new(PACKAGE_B)),
        Err(LedgerError::CorruptRecord)
    ));
    assert!(!record_path(&f.ledger_dir, 2, false).exists());
}

#[test]
fn pending_record_from_aborted_write_forces_manual_repair() {
    let f = Fixture::new();
    let (v, staged, binding) = f.prepare(1, 17, V1, PACKAGE_A);
    let p = record_path(&f.ledger_dir, 1, true);
    fs::write(&p, b"partial write").unwrap();
    let err = f.ledger.propose(
        TrustedFloorHead::genesis(), binding, &staged, &v,
        &mut Cursor::new(PACKAGE_A),
    ).unwrap_err();
    assert!(matches!(err, LedgerError::NeedsRepair));
    assert!(p.exists());
    assert!(!record_path(&f.ledger_dir, 1, false).exists());
}

#[test]
fn truncated_stream_or_forged_stage_receipt_cannot_advance_floor() {
    let f = Fixture::new();
    let (v, mut staged, binding) = f.prepare(1, 17, V1, PACKAGE_A);
    let err = f.ledger.propose(
        TrustedFloorHead::genesis(), binding, &staged, &v,
        &mut Cursor::new(&PACKAGE_A[..5]),
    ).unwrap_err();
    assert!(matches!(err, LedgerError::Release(_)));
    staged.verified_package.package_sha256[0] ^= 1;
    assert!(matches!(
        f.ledger.propose(TrustedFloorHead::genesis(), binding, &staged, &v,
            &mut Cursor::new(PACKAGE_A)),
        Err(LedgerError::PackageMismatch)
    ));
    assert_eq!(fs::read_dir(f.ledger_dir.path()).unwrap().count(), 0);
}

#[test]
fn unsafe_dir_and_symlink_record_are_never_followed() {
    let bad = tempfile::tempdir().unwrap();
    fs::set_permissions(bad.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        FloorLedgerLab::open(bad.path()), Err(LedgerError::UnsafeDirectory)
    ));
    fs::set_permissions(bad.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let f = Fixture::new();
    let (v, staged, binding) = f.prepare(1, 17, V1, PACKAGE_A);
    let p = record_path(&f.ledger_dir, 1, false);
    symlink("/etc/passwd", &p).unwrap();
    assert!(f.ledger.propose(
        TrustedFloorHead::genesis(), binding, &staged, &v,
        &mut Cursor::new(PACKAGE_A),
    ).is_err());
    assert!(p.symlink_metadata().unwrap().file_type().is_symlink());
}

#[test]
fn two_independent_directory_handles_serialize_and_replay_same_proposal() {
    let f = Fixture::new();
    let (verified, staged, binding) = f.prepare(1, 17, V1, PACKAGE_A);
    let ledger_path = Arc::new(f.ledger_dir.path().to_path_buf());
    let threads: Vec<_> = (0..2).map(|_| {
        let path = Arc::clone(&ledger_path);
        let v = verified.clone();
        let stage = staged.clone();
        thread::spawn(move || {
            let ledger = FloorLedgerLab::open(&path).unwrap();
            ledger.propose(
                TrustedFloorHead::genesis(), binding, &stage, &v,
                &mut Cursor::new(PACKAGE_A),
            ).unwrap().idempotent_replay
        })
    }).collect();
    let mut states: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    states.sort();
    assert_eq!(states, [false, true]);
    assert_eq!(fs::read_dir(f.ledger_dir.path()).unwrap().count(), 1);
}
