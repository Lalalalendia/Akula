use super::*;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};

const NOW: u64 = 1_800_000_000;

fn initial() -> (tempfile::TempDir, FirstReleaseApproval) {
    let (dir, approval) = crate::server_root_v3_trust::release_pins::tests::fixture();
    let trust = dir.path().join("trust");
    fs::remove_file(trust.join(FIRST_APPROVAL_FILE)).unwrap();
    fs::remove_file(trust.join(FIRST_APPROVAL_MARKER)).unwrap();
    (dir, approval)
}
fn enroll(
    dir: &tempfile::TempDir,
    approval: &FirstReleaseApproval,
    now: u64,
) -> Result<(), FirstReleaseEnrollError> {
    let trust = File::open(dir.path().join("trust")).unwrap();
    enroll_once_from_reviewed_bytes(&trust, &approval.encode(), now)
}
#[test]
fn first_release_is_single_use_and_reader_accepts_published_signed_inode() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    let (dir, approval) = initial();
    enroll(&dir, &approval, NOW).unwrap();
    let trust = dir.path().join("trust");
    let fd = verify_first_from_trust_fd(&File::open(&trust).unwrap(), NOW).unwrap();
    assert_eq!(fd.receipt.package_sha256, approval.package_sha256);
    assert_eq!(fd.receipt.byte_length, approval.package_size);
    assert_eq!(
        fs::metadata(trust.join(FIRST_APPROVAL_FILE))
            .unwrap()
            .permissions()
            .mode() & 0o7777,
        0o600
    );
    assert!(enroll(&dir, &approval, NOW).is_err());
}
#[test]
fn wrong_release_interrupted_state_and_symlinks_hold_without_new_approval() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    for variant in 0..11 {
        let (dir, mut approval) = initial();
        let trust = dir.path().join("trust");
        let stage = trust.join(FIRST_RELEASE_STAGE);
        match variant {
            0 => approval.package_sha256[0] ^= 1,
            1 => approval.manifest_fingerprint[0] ^= 1,
            2 => approval.package_size += 1,
            3 => approval.sequence += 1,
            4 => { fs::remove_file(stage.join("signature")).unwrap(); }
            5 => { fs::write(stage.join("node"), b"mutated bytes").unwrap(); }
            6 => {
                fs::rename(&stage, trust.join("saved-stage")).unwrap();
                symlink("saved-stage", &stage).unwrap();
            }
            7 => { fs::write(trust.join(APPROVAL_PENDING), b"interrupted").unwrap(); }
            8 => { fs::write(trust.join(FIRST_APPROVAL_MARKER), FIRST_APPROVAL_MARKER_BYTES).unwrap(); }
            9 => {
                let p = trust.join("b0v3.policy");
                let mut bytes = fs::read(&p).unwrap();
                let last = bytes.len() - 1;
                bytes[last] ^= 1;
                fs::write(p, bytes).unwrap();
            }
            10 => {},
            _ => unreachable!(),
        }
        let when = if variant == 10 { NOW + 100 } else { NOW };
        assert!(enroll(&dir, &approval, when).is_err(), "variant {variant}");
        assert!(!trust.join(FIRST_APPROVAL_FILE).exists(), "variant {variant}");
    }
}
#[test]
fn nonroot_cannot_publish_approved_release() {
    if rustix::process::geteuid().as_raw() == 0 {
        return;
    }
    let (dir, approval) = initial();
    assert!(matches!(
        enroll(&dir, &approval, NOW),
        Err(FirstReleaseEnrollError::Release(FirstReleaseError::Trust(
            RootEnrollmentError::NotRoot
        )))
    ));
}
