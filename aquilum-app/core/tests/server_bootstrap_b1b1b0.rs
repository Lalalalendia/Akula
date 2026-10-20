//! B1b1b0: explicit host-owner floor initialization and crash-refusal lab.
#![cfg(all(target_os = "linux", feature = "server-bootstrap-b1b1b0-floor-lab"))]

use aquilum_core::server_approval_lab::{
    ApprovalContext, OperatorApprovalV1, SignedOperatorApproval, TrustedOperatorKey,
};
use aquilum_core::server_floor_journal_lab::{
    InertCommitFault, JournalError, OwnerAnchor, OwnerFloorJournalLab,
};
use aquilum_core::server_release_lab::{
    verify_signed_release, ReleaseFloor, ReleaseManifestV1, SignedRelease,
    TargetPlatform, TrustedReleaseSigner, VerifiedRelease, Version,
};
use aquilum_core::server_staging_lab::{
    ApprovedPlanDigest, InstallId, PrivateStagingLab, StageReceipt,
};
use ring::{digest, signature::{self, KeyPair}};
use std::{fs, io::Cursor, os::unix::fs::{symlink, PermissionsExt}, sync::Arc, thread};
use tempfile::TempDir;

const NOW: u64 = 1_800_000_000;
const ARTIFACT: &[u8] = b"AKULA B1b1b0 synthetic bytes, no execution";
const HOST_KEY: [u8; 32] = [0x73; 32];

fn id() -> InstallId { InstallId([0x11; 16]) }
fn plan() -> ApprovedPlanDigest { ApprovedPlanDigest([0x22; 32]) }

fn release_with_sequence(sequence: u64) -> VerifiedRelease {
    let signer = signature::Ed25519KeyPair::from_seed_unchecked(&[42; 32]).unwrap();
    let manifest = ReleaseManifestV1 {
        platform: TargetPlatform::LinuxX86_64,
        version: Version { major: 0, minor: 4, patch: sequence as u32 },
        release_sequence: sequence,
        package_size: ARTIFACT.len() as u64,
        package_sha256: digest::digest(&digest::SHA256, ARTIFACT)
            .as_ref().try_into().unwrap(),
        not_before_unix: NOW - 60,
        expires_unix: NOW + 3600,
        signer_id: "akula-test-release".into(),
    };
    let sig = signer.sign(&manifest.canonical_signed_bytes());
    let signed = SignedRelease {
        manifest, signature: sig.as_ref().try_into().unwrap(),
    };
    let trust = TrustedReleaseSigner {
        signer_id: "akula-test-release".into(),
        ed25519_public: signer.public_key().as_ref().try_into().unwrap(),
    };
    verify_signed_release(&signed, &trust, ReleaseFloor::initial(), NOW).unwrap()
}

fn staged(release: &VerifiedRelease, install_id: InstallId)
    -> (TempDir, StageReceipt)
{
    let tmp = tempfile::tempdir().unwrap();
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let stage = PrivateStagingLab::open(tmp.path()).unwrap();
    let receipt = stage.stage(
        install_id, plan(), release, &mut Cursor::new(ARTIFACT),
    ).unwrap();
    (tmp, receipt)
}

fn trusted() -> TrustedOperatorKey {
    let key = signature::Ed25519KeyPair::from_seed_unchecked(&[0x35; 32]).unwrap();
    TrustedOperatorKey {
        operator_id: "owner-test".into(),
        ed25519_public: key.public_key().as_ref().try_into().unwrap(),
    }
}

fn sign(release: &VerifiedRelease, install_id: InstallId)
    -> SignedOperatorApproval
{
    let key = signature::Ed25519KeyPair::from_seed_unchecked(&[0x35; 32]).unwrap();
    let floor = release.release_floor_candidate();
    let document = OperatorApprovalV1 {
        operator_id: "owner-test".into(),
        install_id,
        host_key_sha256: HOST_KEY,
        plan_digest: plan(),
        package_sha256: release.manifest().package_sha256,
        signed_manifest_fingerprint: floor.signed_manifest_fingerprint,
        release_sequence: floor.sequence,
        issued_at_unix: NOW - 10,
        expires_at_unix: NOW + 200,
    };
    let signature = key.sign(&document.canonical_signed_bytes());
    SignedOperatorApproval {
        document, signature: signature.as_ref().try_into().unwrap(),
    }
}

fn context<'a>(release: &'a VerifiedRelease, stage: &'a StageReceipt)
    -> ApprovalContext<'a>
{
    ApprovalContext {
        verified_release: release,
        staged_receipt: stage,
        observed_host_key_sha256: HOST_KEY,
        trusted_plan_digest: plan(),
        persisted_floor: ReleaseFloor::initial(),
    }
}

fn fixture() -> (TempDir, OwnerFloorJournalLab, TrustedOperatorKey) {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let store = OwnerFloorJournalLab::open(root.path()).unwrap();
    let operator = trusted();
    (root, store, operator)
}

#[test]
fn explicit_initialization_and_successful_reopen_preserve_floor() {
    let (tmp, store, operator) = fixture();
    assert!(matches!(store.snapshot(), Err(JournalError::Uninitialized)));
    let anchor = OwnerAnchor::from_trusted_operator(HOST_KEY, &operator);
    assert_eq!(store.initialize(anchor).unwrap().floor, ReleaseFloor::initial());
    let release = release_with_sequence(18);
    let (_stage_dir, receipt) = staged(&release, id());
    let signed = sign(&release, id());
    let accepted = store.record(&signed, &operator, context(&release, &receipt), NOW).unwrap();
    assert_eq!(accepted.floor.sequence, 18);
    drop(store);
    let reopened = OwnerFloorJournalLab::open(tmp.path()).unwrap();
    assert_eq!(reopened.snapshot().unwrap(), accepted);
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
}

#[test]
fn exact_replayed_approval_cannot_obtain_second_success() {
    let (_tmp, store, operator) = fixture();
    store.initialize(OwnerAnchor::from_trusted_operator(HOST_KEY, &operator)).unwrap();
    let release = release_with_sequence(18);
    let (_stage_dir, receipt) = staged(&release, id());
    let signed = sign(&release, id());
    store.record(&signed, &operator, context(&release, &receipt), NOW).unwrap();
    assert!(matches!(
        store.record(&signed, &operator, context(&release, &receipt), NOW),
        Err(JournalError::ReplayRefused)
    ));
}

#[test]
fn newer_release_may_advance_floor_without_resetting_identity() {
    let (_tmp, store, operator) = fixture();
    let anchor = OwnerAnchor::from_trusted_operator(HOST_KEY, &operator);
    store.initialize(anchor).unwrap();
    let first = release_with_sequence(18);
    let (_stage1, receipt1) = staged(&first, id());
    store.record(&sign(&first, id()), &operator, context(&first, &receipt1), NOW).unwrap();
    let second_id = InstallId([0x12; 16]);
    let second = release_with_sequence(19);
    let (_stage2, receipt2) = staged(&second, second_id);
    let accepted = store.record(
        &sign(&second, second_id), &operator, context(&second, &receipt2), NOW,
    ).unwrap();
    assert_eq!(accepted.floor.sequence, 19);
    assert_eq!(accepted.anchor, anchor);
    assert_eq!(accepted.last_install_id, second_id);
}

#[test]
fn older_release_after_update_refuses_rollback() {
    let (_tmp, store, operator) = fixture();
    store.initialize(OwnerAnchor::from_trusted_operator(HOST_KEY, &operator)).unwrap();
    let latest = release_with_sequence(19);
    let (_stage1, receipt1) = staged(&latest, id());
    store.record(&sign(&latest, id()), &operator, context(&latest, &receipt1), NOW).unwrap();
    let older = release_with_sequence(18);
    let different_id = InstallId([0x22; 16]);
    let (_stage2, receipt2) = staged(&older, different_id);
    let err = store.record(
        &sign(&older, different_id), &operator, context(&older, &receipt2), NOW,
    ).unwrap_err();
    assert!(matches!(err, JournalError::Approval(_)));
    assert_eq!(store.snapshot().unwrap().floor.sequence, 19);
}

#[test]
fn substituted_host_or_operator_is_rejected_without_writes() {
    let (_tmp, store, operator) = fixture();
    let original = store.initialize(
        OwnerAnchor::from_trusted_operator(HOST_KEY, &operator),
    ).unwrap();
    let release = release_with_sequence(18);
    let (_stage_dir, receipt) = staged(&release, id());
    let signed = sign(&release, id());
    let mut different_host = context(&release, &receipt);
    different_host.observed_host_key_sha256 = [0x55; 32];
    assert!(matches!(
        store.record(&signed, &operator, different_host, NOW),
        Err(JournalError::AnchorMismatch)
    ));
    let mut other_operator = operator.clone();
    other_operator.ed25519_public = [0x88; 32];
    assert!(matches!(
        store.record(&signed, &other_operator, context(&release, &receipt), NOW),
        Err(JournalError::AnchorMismatch)
    ));
    assert_eq!(store.snapshot().unwrap(), original);
}

#[test]
fn tampered_signature_refused_and_floor_unchanged() {
    let (_tmp, store, operator) = fixture();
    let original = store.initialize(
        OwnerAnchor::from_trusted_operator(HOST_KEY, &operator),
    ).unwrap();
    let release = release_with_sequence(18);
    let (_stage_dir, receipt) = staged(&release, id());
    let mut forged = sign(&release, id());
    forged.signature[0] ^= 1;
    assert!(matches!(
        store.record(&forged, &operator, context(&release, &receipt), NOW),
        Err(JournalError::Approval(_))
    ));
    assert_eq!(store.snapshot().unwrap(), original);
}

#[test]
fn leftover_pending_after_interruption_blocks_all_replays() {
    let (tmp, store, operator) = fixture();
    let original = store.initialize(
        OwnerAnchor::from_trusted_operator(HOST_KEY, &operator),
    ).unwrap();
    let release = release_with_sequence(18);
    let (_stage_dir, receipt) = staged(&release, id());
    let err = store.record_with_fault(
        &sign(&release, id()), &operator, context(&release, &receipt), NOW,
        InertCommitFault::AfterPendingSync,
    ).unwrap_err();
    assert!(matches!(err, JournalError::InjectedInterruption));
    assert!(tmp.path().join("owner-floor.v1.pending").exists());
    assert!(matches!(store.snapshot(), Err(JournalError::NeedsRepair)));
    assert!(matches!(
        store.record(&sign(&release, id()), &operator, context(&release, &receipt), NOW),
        Err(JournalError::NeedsRepair)
    ));
    // The old accepted state was not modified; the pending attempt is held.
    assert_ne!(fs::read(tmp.path().join("owner-floor.v1")).unwrap(),
        fs::read(tmp.path().join("owner-floor.v1.pending")).unwrap());
    assert_eq!(original.floor, ReleaseFloor::initial());
}

#[test]
fn corruption_or_deletion_never_auto_resets_floor() {
    let (tmp, store, operator) = fixture();
    store.initialize(OwnerAnchor::from_trusted_operator(HOST_KEY, &operator)).unwrap();
    fs::write(tmp.path().join("owner-floor.v1"), b"truncated").unwrap();
    assert!(matches!(store.snapshot(), Err(JournalError::CorruptState)));
    fs::remove_file(tmp.path().join("owner-floor.v1")).unwrap();
    assert!(matches!(store.snapshot(), Err(JournalError::Uninitialized)));
}

#[test]
fn rejected_initialization_replay_and_invalid_anchor() {
    let (_tmp, store, operator) = fixture();
    assert!(matches!(
        store.initialize(OwnerAnchor {
            host_key_sha256: [0; 32], operator_public_key_sha256: [0x11; 32],
        }),
        Err(JournalError::InvalidAnchor)
    ));
    let anchor = OwnerAnchor::from_trusted_operator(HOST_KEY, &operator);
    store.initialize(anchor).unwrap();
    assert!(matches!(store.initialize(anchor), Err(JournalError::AlreadyInitialized)));
}

#[test]
fn unsafe_directory_and_symlink_are_never_followed() {
    let tmp = tempfile::tempdir().unwrap();
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(OwnerFloorJournalLab::open(tmp.path()),
        Err(JournalError::UnsafeDirectory)));
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let store = OwnerFloorJournalLab::open(tmp.path()).unwrap();
    let operator = trusted();
    store.initialize(OwnerAnchor::from_trusted_operator(HOST_KEY, &operator)).unwrap();
    let state_path = tmp.path().join("owner-floor.v1");
    fs::remove_file(&state_path).unwrap();
    symlink("/etc/passwd", &state_path).unwrap();
    assert!(matches!(store.snapshot(), Err(JournalError::UnsafeEntry)));
}

#[test]
fn independent_handles_serialize_approval_commit() {
    let (tmp, store, operator) = fixture();
    store.initialize(OwnerAnchor::from_trusted_operator(HOST_KEY, &operator)).unwrap();
    let root = Arc::new(tmp.path().to_path_buf());
    let outcomes: Vec<_> = (0..2).map(|_| {
        let root = Arc::clone(&root);
        thread::spawn(move || {
            let local = OwnerFloorJournalLab::open(&root).unwrap();
            let release = release_with_sequence(18);
            let (_stage_dir, receipt) = staged(&release, id());
            let result = local.record(&sign(&release, id()), &trusted(),
                context(&release, &receipt), NOW);
            result.map(|s| s.floor.sequence).map_err(|e| match e {
                JournalError::ReplayRefused => "replay",
                other => { eprintln!("{other:?}"); "unexpected" },
            })
        })
    }).map(|j| j.join().unwrap()).collect();
    assert_eq!(outcomes.iter().filter(|x| x.is_ok()).count(), 1);
    assert_eq!(outcomes.iter().filter(|x| **x == Err("replay")).count(), 1);
}

#[test]
fn injected_pre_dirsync_is_never_a_success_receipt() {
    let (_tmp, store, operator) = fixture();
    store.initialize(OwnerAnchor::from_trusted_operator(HOST_KEY, &operator)).unwrap();
    let release = release_with_sequence(18);
    let (_stage_dir, receipt) = staged(&release, id());
    assert!(matches!(
        store.record_with_fault(
            &sign(&release, id()), &operator, context(&release, &receipt), NOW,
            InertCommitFault::AfterRenameBeforeDirectorySync,
        ),
        Err(JournalError::InjectedInterruption)
    ));
    // Simulated fault proves no success receipt, NOT a physical powercut result.
}
