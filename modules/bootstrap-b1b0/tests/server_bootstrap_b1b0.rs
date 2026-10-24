//! B1b0: disposable private-directory custody proof only.
//! No SSH, root, systemd, package execution or user vaults.
#![cfg(target_os = "linux")]

use akula_server_staging_b1b0::server_release_lab::{
    verify_signed_release, ReleaseFloor, ReleaseManifestV1, SignedRelease,
    TargetPlatform, TrustedReleaseSigner, VerifiedRelease, Version,
};
use akula_server_staging_b1b0::server_staging_lab::{
    ApprovedPlanDigest, InstallId, PrivateStagingLab, StageError,
};
use ring::{digest, signature::{self, KeyPair}};
use std::{fs, io::Cursor, os::unix::fs::{symlink, PermissionsExt}, sync::Arc, thread};
use tempfile::TempDir;

const NOW: u64 = 1_800_000_000;
const PACKAGE: &[u8] = b"AKULA B1b0 inert synthetic artifact; DO NOT execute";

fn authenticated(content: &[u8]) -> VerifiedRelease {
    // Public, reproducible TEST key. Not a trusted Akula release authority.
    let key = signature::Ed25519KeyPair::from_seed_unchecked(&[42u8; 32]).unwrap();
    let manifest = ReleaseManifestV1 {
        platform: TargetPlatform::LinuxX86_64,
        version: Version { major: 0, minor: 3, patch: 9 },
        release_sequence: 17,
        package_size: content.len() as u64,
        package_sha256: digest::digest(&digest::SHA256, content)
            .as_ref().try_into().unwrap(),
        not_before_unix: NOW - 1,
        expires_unix: NOW + 3600,
        signer_id: "akula-node-test-release".into(),
    };
    let signature = key.sign(&manifest.canonical_signed_bytes());
    let signed = SignedRelease {
        manifest, signature: signature.as_ref().try_into().unwrap(),
    };
    let trust = TrustedReleaseSigner {
        signer_id: "akula-node-test-release".into(),
        ed25519_public: key.public_key().as_ref().try_into().unwrap(),
    };
    verify_signed_release(&signed, &trust, ReleaseFloor::initial(), NOW).unwrap()
}

fn lab() -> (TempDir, PrivateStagingLab) {
    let tmp = tempfile::tempdir().unwrap();
    // The test framework may create tempdirs with inherited 0755 mode.
    // Set the explicit 0700 directory guarantee required by the real lab.
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let stage = PrivateStagingLab::open(tmp.path()).unwrap();
    (tmp, stage)
}

fn id() -> InstallId { InstallId([0x11; 16]) }
fn approval() -> ApprovedPlanDigest { ApprovedPlanDigest([0x22; 32]) }

#[test]
fn exact_verified_package_and_plan_are_idempotent_without_execution() {
    let (tmp, stage) = lab();
    let verified = authenticated(PACKAGE);
    let first = stage.stage(id(), approval(), &verified, &mut Cursor::new(PACKAGE)).unwrap();
    assert!(!first.idempotent_replay);
    assert_eq!(first.verified_package.byte_length, PACKAGE.len() as u64);
    let second = stage.stage(id(), approval(), &verified, &mut Cursor::new([])).unwrap();
    assert!(second.idempotent_replay);
    assert_eq!(first.verified_package, second.verified_package);
    let names: Vec<_> = fs::read_dir(tmp.path()).unwrap().collect();
    assert_eq!(names.len(), 2); // one .verified artifact and one .journal
    assert_eq!(
        fs::read(tmp.path().join("11111111111111111111111111111111.verified")).unwrap(),
        PACKAGE
    );
}

#[test]
fn a_different_approval_digest_refuses_same_install_id() {
    // Keep the TempDir alive until the stage is dropped.
    let (_tmp, stage) = lab();
    let verified = authenticated(PACKAGE);
    stage.stage(id(), approval(), &verified, &mut Cursor::new(PACKAGE)).unwrap();
    let err = stage.stage(id(), ApprovedPlanDigest([0x23; 32]),
        &verified, &mut Cursor::new(PACKAGE)).unwrap_err();
    assert!(matches!(err, StageError::JournalConflict));
}

#[test]
fn changed_package_or_truncation_leave_no_accepted_journal() {
    let (tmp, stage) = lab();
    let verified = authenticated(PACKAGE);
    let mut altered = PACKAGE.to_vec();
    altered[0] ^= 1;
    let err = stage.stage(id(), approval(), &verified,
        &mut Cursor::new(altered)).unwrap_err();
    assert!(matches!(err, StageError::Release(_)));
    let journal = tmp.path().join("11111111111111111111111111111111.journal");
    assert!(!journal.exists());
    let retry = stage.stage(id(), approval(), &verified,
        &mut Cursor::new(PACKAGE)).unwrap_err();
    assert!(matches!(retry, StageError::NeedsRepair));
}

#[test]
fn incomplete_orphan_file_refuses_automatic_cleanup() {
    let (tmp, stage) = lab();
    let path = tmp.path().join("11111111111111111111111111111111.verified");
    fs::write(&path, PACKAGE).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
    let err = stage.stage(id(), approval(), &authenticated(PACKAGE),
        &mut Cursor::new(PACKAGE)).unwrap_err();
    assert!(matches!(err, StageError::NeedsRepair));
    assert!(path.exists()); // do not destroy uncommitted data
}

#[test]
fn journal_corruption_refuses_idempotent_replay() {
    let (tmp, stage) = lab();
    let verified = authenticated(PACKAGE);
    stage.stage(id(), approval(), &verified, &mut Cursor::new(PACKAGE)).unwrap();
    let journal = tmp.path().join("11111111111111111111111111111111.journal");
    fs::write(&journal, b"corrupted").unwrap();
    let err = stage.stage(id(), approval(), &verified,
        &mut Cursor::new(PACKAGE)).unwrap_err();
    assert!(matches!(err, StageError::JournalConflict));
}

#[test]
fn untrusted_private_dir_or_symlink_entry_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        PrivateStagingLab::open(tmp.path()),
        Err(StageError::UnsafeDirectory)
    ));
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let stage = PrivateStagingLab::open(tmp.path()).unwrap();
    let link = tmp.path().join("11111111111111111111111111111111.pending");
    symlink("/etc/passwd", &link).unwrap();
    assert!(stage.stage(id(), approval(), &authenticated(PACKAGE),
        &mut Cursor::new(PACKAGE)).is_err());
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
}

#[test]
fn invalid_install_or_approval_is_refused_before_writes() {
    let (tmp, stage) = lab();
    let verified = authenticated(PACKAGE);
    assert!(matches!(
        stage.stage(InstallId([0; 16]), approval(), &verified, &mut Cursor::new(PACKAGE)),
        Err(StageError::InvalidInstallId)
    ));
    assert!(matches!(
        stage.stage(id(), ApprovedPlanDigest([0; 32]),
            &verified, &mut Cursor::new(PACKAGE)),
        Err(StageError::InvalidApproval)
    ));
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
}

#[test]
fn independent_handles_serialize_writers() {
    let tmp = tempfile::tempdir().unwrap();
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let root = Arc::new(tmp.path().to_path_buf());
    let handles: Vec<_> = (0..2).map(|_| {
        let root = Arc::clone(&root);
        thread::spawn(move || {
            let stage = PrivateStagingLab::open(&root).unwrap();
            stage.stage(id(), approval(), &authenticated(PACKAGE),
                &mut Cursor::new(PACKAGE)).unwrap().idempotent_replay
        })
    }).collect();
    let mut results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    results.sort();
    assert_eq!(results, [false, true]);
}
