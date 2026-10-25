use super::*;
use std::fs::{self, Permissions};
use std::os::unix::fs::{symlink, PermissionsExt};

const NOW: u64 = 1_800_000_000;

fn fixture() -> (tempfile::TempDir, FirstReleaseApproval) {
    let (dir, approval) = crate::server_root_v3_trust::release_pins::tests::fixture();
    let trust = dir.path().join("trust");
    fs::remove_file(trust.join(FIRST_APPROVAL_FILE)).unwrap();
    fs::remove_file(trust.join(FIRST_APPROVAL_MARKER)).unwrap();
    let admin = dir.path().join("root");
    fs::create_dir(&admin).unwrap();
    fs::set_permissions(&admin, Permissions::from_mode(0o700)).unwrap();
    fs::rename(
        trust.join(FIRST_RELEASE_STAGE),
        admin.join(ADMIN_SOURCE),
    )
    .unwrap();
    fs::write(admin.join(ADMIN_CANDIDATE), approval.encode()).unwrap();
    fs::set_permissions(
        admin.join(ADMIN_CANDIDATE),
        Permissions::from_mode(0o600),
    )
    .unwrap();
    (dir, approval)
}
fn stage(dir: &tempfile::TempDir, now: u64) -> Result<(), FirstStageError> {
    let trust = File::open(dir.path().join("trust")).unwrap();
    let admin = File::open(dir.path().join("root")).unwrap();
    stage_from_admin_fds(&trust, &admin, now)
}

#[test]
fn signed_b1a_stage_is_durable_one_shot_and_still_not_approved() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    let (dir, approval) = fixture();
    stage(&dir, NOW).unwrap();
    let trust = dir.path().join("trust");
    assert!(trust.join(FIRST_RELEASE_STAGE).exists());
    assert!(!trust.join(STAGE_PENDING).exists());
    assert!(!trust.join(FIRST_APPROVAL_MARKER).exists());
    assert!(stage(&dir, NOW).is_err(), "staging replay must HOLD");
    // Signed data alone cannot satisfy the release approval reader.
    let root_fd = File::open(&trust).unwrap();
    assert!(verify_first_from_trust_fd(&root_fd, NOW).is_err());
    // The separate one-use root approval is the only follow-up that unlocks
    // the read-only B1a descriptor. No Node /opt write or systemd starts.
    super::super::enrollment::enroll_once_from_reviewed_bytes(
        &root_fd,
        &approval.encode(),
        NOW,
    )
    .unwrap();
    let verified = verify_first_from_trust_fd(&root_fd, NOW).unwrap();
    assert_eq!(verified.receipt.package_sha256, approval.package_sha256);
}

#[test]
fn invalid_or_interrupted_stage_never_publishes_a_new_signed_release() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    for variant in 0..11 {
        let (dir, mut approval) = fixture();
        let trust = dir.path().join("trust");
        let home = dir.path().join("root");
        let source = home.join(ADMIN_SOURCE);
        match variant {
            0 => approval.package_sha256[0] ^= 1,
            1 => approval.manifest_fingerprint[0] ^= 1,
            2 => approval.sequence += 1,
            3 => {
                fs::remove_file(source.join("signature")).unwrap();
            }
            4 => {
                fs::write(source.join("node"), b"changed node data").unwrap();
            }
            5 => {
                fs::rename(&source, home.join("other-source")).unwrap();
                symlink("other-source", &source).unwrap();
            }
            6 => {
                fs::set_permissions(&source, Permissions::from_mode(0o755)).unwrap();
            }
            7 => {
                fs::create_dir(trust.join(STAGE_PENDING)).unwrap();
            }
            8 => {
                fs::create_dir(trust.join(FIRST_RELEASE_STAGE)).unwrap();
            }
            9 => {
                fs::write(trust.join(FIRST_APPROVAL_MARKER), FIRST_APPROVAL_MARKER_BYTES).unwrap();
            }
            10 => {}
            _ => unreachable!(),
        }
        if variant < 3 {
            fs::write(home.join(ADMIN_CANDIDATE), approval.encode()).unwrap();
        }
        let now = if variant == 10 { NOW + 100 } else { NOW };
        assert!(stage(&dir, now).is_err(), "invalid stage variant {variant}");
        assert!(!trust.join(FIRST_APPROVAL_FILE).exists());
        if variant != 8 {
            assert!(!trust.join(FIRST_RELEASE_STAGE).exists());
        }
    }
}

#[test]
fn unprivileged_source_cannot_stage_or_enroll_a_release() {
    if rustix::process::geteuid().as_raw() == 0 {
        return;
    }
    let (dir, _) = fixture();
    assert!(matches!(
        stage(&dir, NOW),
        Err(FirstStageError::Release(FirstReleaseError::Trust(
            RootEnrollmentError::NotRoot
        )))
    ));
    assert!(!dir.path().join("trust").join(FIRST_RELEASE_STAGE).exists());
}
