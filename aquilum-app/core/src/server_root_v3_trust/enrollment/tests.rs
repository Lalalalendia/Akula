use super::*;
use std::fs::{self, Permissions};
use std::os::unix::fs::{symlink, PermissionsExt};
use tempfile::tempdir;

fn policy() -> Vec<u8> {
    let mut b = DOMAIN.to_vec();
    b.push(3);
    b.push(10);
    b.extend_from_slice(b"owner-test");
    b.extend_from_slice(&[0x11; 32]);
    b.extend_from_slice(&[0x22; 32]);
    b.extend_from_slice(&[0x33; 32]);
    b.push(12);
    b.extend_from_slice(b"release-test");
    b.extend_from_slice(&[0x44; 32]);
    b
}
fn setup(root: &Path, enrolled: bool) -> File {
    fs::create_dir(root).unwrap();
    fs::set_permissions(root, Permissions::from_mode(0o700)).unwrap();
    if enrolled {
        fs::write(root.join("enrollment"), WITNESS_ENROLLMENT).unwrap();
        fs::set_permissions(root.join("enrollment"), Permissions::from_mode(0o600)).unwrap();
    }
    File::open(root).unwrap()
}
fn empty(root: &Path) -> bool {
    for leaf in [POLICY_PENDING, POLICY_FINAL, MARKER_PENDING, MARKER_FINAL] {
        if root.join(leaf).exists() {
            return false;
        }
    }
    true
}

#[test]
fn first_enrollment_is_root_only_and_single_use() {
    let dir = tempdir().unwrap();
    let trust = setup(&dir.path().join("trust"), true);
    let bytes = policy();
    if rustix::process::geteuid().as_raw() != 0 {
        assert!(matches!(
            enroll_once_from_admin_bytes(&trust, &bytes),
            Err(AdminEnrollError::Read(RootEnrollmentError::NotRoot)),
        ));
        assert!(empty(&dir.path().join("trust")));
        return;
    }
    enroll_once_from_admin_bytes(&trust, &bytes).unwrap();
    let checked = RootEnrolledV3::from_trust_fd(&trust).unwrap();
    assert_eq!(checked.owner_id, "owner-test");
    assert_eq!(checked.release_pub, [0x44; 32]);
    assert_eq!(
        fs::read(dir.path().join("trust").join(POLICY_FINAL)).unwrap(),
        bytes,
    );
    assert!(empty(&dir.path().join("trust")) == false);
    for name in [POLICY_FINAL, MARKER_FINAL] {
        assert_eq!(
            fs::metadata(dir.path().join("trust").join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o600,
        );
    }
    assert!(!dir.path().join("trust").join(POLICY_PENDING).exists());
    assert!(!dir.path().join("trust").join(MARKER_PENDING).exists());
    // A lost acknowledgement may never be treated as a second grant.
    assert!(matches!(
        enroll_once_from_admin_bytes(&trust, &bytes),
        Err(AdminEnrollError::IncompleteOrAlreadyEnrolled),
    ));
}

#[test]
fn absent_witness_or_wrong_candidate_rejects_before_any_publication() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    let dir = tempdir().unwrap();
    let root = dir.path().join("trust");
    let trust = setup(&root, false);
    assert!(enroll_once_from_admin_bytes(&trust, &policy()).is_err());
    assert!(empty(&root));
    fs::write(root.join("enrollment"), WITNESS_ENROLLMENT).unwrap();
    fs::set_permissions(root.join("enrollment"), Permissions::from_mode(0o600)).unwrap();
    let mut broken = policy();
    broken[0] ^= 1;
    assert!(enroll_once_from_admin_bytes(&trust, &broken).is_err());
    assert!(empty(&root));
    let mut broken_key = policy();
    let owner_key_offset = DOMAIN.len() + 1 + 1 + 10;
    broken_key[owner_key_offset..owner_key_offset + 32].fill(0);
    assert!(enroll_once_from_admin_bytes(&trust, &broken_key).is_err());
    assert!(empty(&root));
}

#[test]
fn interrupted_or_foreign_enrollment_is_hold_without_auto_repair() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    let dir = tempdir().unwrap();
    let root = dir.path().join("trust");
    let trust = setup(&root, true);
    fs::write(root.join(POLICY_PENDING), b"incomplete").unwrap();
    assert!(matches!(
        enroll_once_from_admin_bytes(&trust, &policy()),
        Err(AdminEnrollError::IncompleteOrAlreadyEnrolled),
    ));
    assert!(!root.join(POLICY_FINAL).exists());
    assert!(!root.join(MARKER_FINAL).exists());
    fs::remove_file(root.join(POLICY_PENDING)).unwrap();
    symlink("untrusted", root.join(POLICY_FINAL)).unwrap();
    assert!(matches!(
        enroll_once_from_admin_bytes(&trust, &policy()),
        Err(AdminEnrollError::IncompleteOrAlreadyEnrolled),
    ));
    assert!(!root.join(MARKER_FINAL).exists());
}
