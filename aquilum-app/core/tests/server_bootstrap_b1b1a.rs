//! B1b1a: signed owner authorization is inert and scoped to one staged release.
#![cfg(all(target_os = "linux", feature = "server-bootstrap-b1b1-approval-lab"))]

use aquilum_core::server_approval_lab::{
    verify_operator_approval, ApprovalContext, ApprovalError, OperatorApprovalV1,
    SignedOperatorApproval, TrustedOperatorKey,
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
const ARTIFACT: &[u8] = b"AKULA B1b1a owner approval: inert test bytes";
const HOST_KEY: [u8; 32] = [0x73; 32];

fn install_id() -> InstallId { InstallId([0x11; 16]) }
fn plan() -> ApprovedPlanDigest { ApprovedPlanDigest([0x22; 32]) }

fn authenticated_release() -> VerifiedRelease {
    let signer = signature::Ed25519KeyPair::from_seed_unchecked(&[42; 32]).unwrap();
    let manifest = ReleaseManifestV1 {
        platform: TargetPlatform::LinuxX86_64,
        version: Version { major: 0, minor: 4, patch: 0 },
        release_sequence: 18,
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

fn prepared_stage(release: &VerifiedRelease) -> (TempDir, StageReceipt) {
    let tmp = tempfile::tempdir().unwrap();
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let lab = PrivateStagingLab::open(tmp.path()).unwrap();
    let staged = lab.stage(install_id(), plan(), release, &mut Cursor::new(ARTIFACT)).unwrap();
    (tmp, staged)
}

fn sign_approval(document: OperatorApprovalV1) -> (SignedOperatorApproval, TrustedOperatorKey) {
    let signer = signature::Ed25519KeyPair::from_seed_unchecked(&[0x35; 32]).unwrap();
    let sig = signer.sign(&document.canonical_signed_bytes());
    (
        SignedOperatorApproval { document, signature: sig.as_ref().try_into().unwrap() },
        TrustedOperatorKey {
            operator_id: "owner-test".into(),
            ed25519_public: signer.public_key().as_ref().try_into().unwrap(),
        },
    )
}

fn document(release: &VerifiedRelease) -> OperatorApprovalV1 {
    let floor = release.release_floor_candidate();
    OperatorApprovalV1 {
        operator_id: "owner-test".into(),
        install_id: install_id(),
        host_key_sha256: HOST_KEY,
        plan_digest: plan(),
        package_sha256: release.manifest().package_sha256,
        signed_manifest_fingerprint: floor.signed_manifest_fingerprint,
        release_sequence: floor.sequence,
        issued_at_unix: NOW - 10,
        expires_at_unix: NOW + 200,
    }
}

fn ctx<'a>(release: &'a VerifiedRelease, staged: &'a StageReceipt) -> ApprovalContext<'a> {
    ApprovalContext {
        verified_release: release,
        staged_receipt: staged,
        observed_host_key_sha256: HOST_KEY,
        trusted_plan_digest: plan(),
        persisted_floor: ReleaseFloor::initial(),
    }
}

#[test]
fn correctly_signed_scope_is_inert_and_stable() {
    let release = authenticated_release();
    let (_tmp, staged) = prepared_stage(&release);
    let (signed, trust) = sign_approval(document(&release));
    let first = verify_operator_approval(&signed, &trust, ctx(&release, &staged), NOW).unwrap();
    let second = verify_operator_approval(&signed, &trust, ctx(&release, &staged), NOW).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.install_id, install_id());
    assert_eq!(first.approved_plan_digest, plan());
    assert_eq!(first.proposed_floor, release.release_floor_candidate());
}

#[test]
fn wrong_real_host_key_refuses_replayed_approval() {
    let release = authenticated_release();
    let (_tmp, staged) = prepared_stage(&release);
    let (signed, trust) = sign_approval(document(&release));
    let mut context = ctx(&release, &staged);
    context.observed_host_key_sha256 = [0x91; 32];
    assert!(matches!(
        verify_operator_approval(&signed, &trust, context, NOW),
        Err(ApprovalError::ScopeMismatch)
    ));
}

#[test]
fn plan_replacement_is_refused_even_with_genuine_owner_signature() {
    let release = authenticated_release();
    let (_tmp, staged) = prepared_stage(&release);
    let mut altered = document(&release);
    altered.plan_digest = ApprovedPlanDigest([0x42; 32]);
    let (signed, trust) = sign_approval(altered);
    assert!(matches!(
        verify_operator_approval(&signed, &trust, ctx(&release, &staged), NOW),
        Err(ApprovalError::ScopeMismatch)
    ));
}

#[test]
fn edited_signature_is_refused() {
    let release = authenticated_release();
    let (_tmp, staged) = prepared_stage(&release);
    let (mut signed, trust) = sign_approval(document(&release));
    signed.signature[0] ^= 0x80;
    assert!(matches!(
        verify_operator_approval(&signed, &trust, ctx(&release, &staged), NOW),
        Err(ApprovalError::BadSignature)
    ));
}

#[test]
fn unrelated_operator_key_or_id_is_refused() {
    let release = authenticated_release();
    let (_tmp, staged) = prepared_stage(&release);
    let (signed, mut trust) = sign_approval(document(&release));
    trust.operator_id = "different-owner".into();
    assert!(matches!(
        verify_operator_approval(&signed, &trust, ctx(&release, &staged), NOW),
        Err(ApprovalError::UntrustedOperator)
    ));
    trust.operator_id = "owner-test".into();
    trust.ed25519_public = [0x99; 32];
    assert!(matches!(
        verify_operator_approval(&signed, &trust, ctx(&release, &staged), NOW),
        Err(ApprovalError::BadSignature)
    ));
}

#[test]
fn forged_stage_receipt_cannot_change_install_or_bytes() {
    let release = authenticated_release();
    let (_tmp, mut staged) = prepared_stage(&release);
    let (signed, trust) = sign_approval(document(&release));
    staged.install_id = InstallId([0x21; 16]);
    assert!(matches!(
        verify_operator_approval(&signed, &trust, ctx(&release, &staged), NOW),
        Err(ApprovalError::ScopeMismatch)
    ));
    staged.install_id = install_id();
    staged.verified_package.package_sha256[0] ^= 1;
    assert!(matches!(
        verify_operator_approval(&signed, &trust, ctx(&release, &staged), NOW),
        Err(ApprovalError::ScopeMismatch)
    ));
}

#[test]
fn package_manifest_replacement_with_same_payload_is_refused() {
    let release = authenticated_release();
    let (_tmp, staged) = prepared_stage(&release);
    let mut swapped = document(&release);
    swapped.signed_manifest_fingerprint = [0x71; 32];
    let (signed, trust) = sign_approval(swapped);
    assert!(matches!(
        verify_operator_approval(&signed, &trust, ctx(&release, &staged), NOW),
        Err(ApprovalError::ScopeMismatch)
    ));
}

#[test]
fn expiration_future_and_invalid_lifetime_fail_closed() {
    let release = authenticated_release();
    let (_tmp, staged) = prepared_stage(&release);
    let doc = document(&release);
    let (signed, trust) = sign_approval(doc.clone());
    assert!(matches!(
        verify_operator_approval(&signed, &trust, ctx(&release, &staged), NOW + 200),
        Err(ApprovalError::Expired)
    ));
    let mut future = doc.clone();
    future.issued_at_unix = NOW + 30;
    future.expires_at_unix = NOW + 40;
    let (signed, trust) = sign_approval(future);
    assert!(matches!(
        verify_operator_approval(&signed, &trust, ctx(&release, &staged), NOW),
        Err(ApprovalError::NotYetValid)
    ));
    let mut too_long = doc;
    too_long.expires_at_unix = NOW + 3600;
    let (signed, trust) = sign_approval(too_long);
    assert!(matches!(
        verify_operator_approval(&signed, &trust, ctx(&release, &staged), NOW),
        Err(ApprovalError::InvalidMetadata)
    ));
}

#[test]
fn invalid_zero_key_or_plan_is_refused() {
    let release = authenticated_release();
    let (_tmp, staged) = prepared_stage(&release);
    let mut invalid = document(&release);
    invalid.host_key_sha256 = [0; 32];
    let (signed, trust) = sign_approval(invalid);
    assert!(matches!(
        verify_operator_approval(&signed, &trust, ctx(&release, &staged), NOW),
        Err(ApprovalError::InvalidMetadata)
    ));
    let mut invalid = document(&release);
    invalid.plan_digest = ApprovedPlanDigest([0; 32]);
    let (signed, trust) = sign_approval(invalid);
    assert!(matches!(
        verify_operator_approval(&signed, &trust, ctx(&release, &staged), NOW),
        Err(ApprovalError::InvalidMetadata)
    ));
}

#[test]
fn newer_persisted_floor_refuses_old_prepared_release() {
    let release = authenticated_release();
    let (_tmp, staged) = prepared_stage(&release);
    let (signed, trust) = sign_approval(document(&release));
    let mut context = ctx(&release, &staged);
    context.persisted_floor.sequence = release.manifest().release_sequence + 1;
    assert!(matches!(
        verify_operator_approval(&signed, &trust, context, NOW),
        Err(ApprovalError::RollbackRefused)
    ));
}

#[test]
fn same_sequence_with_different_signed_manifest_is_rejected() {
    let release = authenticated_release();
    let (_tmp, staged) = prepared_stage(&release);
    let (signed, trust) = sign_approval(document(&release));
    let mut context = ctx(&release, &staged);
    context.persisted_floor = release.release_floor_candidate();
    context.persisted_floor.signed_manifest_fingerprint[0] ^= 1;
    assert!(matches!(
        verify_operator_approval(&signed, &trust, context, NOW),
        Err(ApprovalError::RollbackRefused)
    ));
}

#[test]
fn exact_same_floor_can_repeat_without_new_install_authority() {
    let release = authenticated_release();
    let (_tmp, staged) = prepared_stage(&release);
    let (signed, trust) = sign_approval(document(&release));
    let mut context = ctx(&release, &staged);
    context.persisted_floor = release.release_floor_candidate();
    assert!(verify_operator_approval(&signed, &trust, context, NOW).is_ok());
}
