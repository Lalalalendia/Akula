use super::*;
use std::fs::{self, Permissions};
use std::os::unix::fs::{symlink, PermissionsExt};
use tempfile::tempdir;

fn encoded_policy() -> Vec<u8> {
    let mut value = DOMAIN.to_vec();
    value.push(3);
    value.push(10);
    value.extend_from_slice(b"owner-test");
    value.extend_from_slice(&[0x11; 32]);
    value.extend_from_slice(&[0x22; 32]);
    value.extend_from_slice(&[0x33; 32]);
    value.push(12);
    value.extend_from_slice(b"release-test");
    value.extend_from_slice(&[0x44; 32]);
    value
}

fn write_root_0600(path: &Path, contents: &[u8]) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn canonical_policy_decoding_refuses_unknown_bytes_and_zero_key() {
    let body = encoded_policy();
    let parsed = RootEnrolledV3::parse(&body).unwrap();
    assert_eq!(parsed.owner_id, "owner-test");
    assert_eq!(parsed.owner_pub, [0x11; 32]);
    assert_eq!(parsed.ssh_host_sha256, [0x22; 32]);
    assert_eq!(parsed.helper_sha256, [0x33; 32]);
    assert_eq!(parsed.release_signer_id, "release-test");
    assert_eq!(parsed.release_pub, [0x44; 32]);
    for variant in 0..7 {
        let mut invalid = body.clone();
        match variant {
            0 => invalid.push(0),
            1 => invalid[DOMAIN.len()] = 2,
            2 => invalid.pop().map(|_| ()).unwrap(),
            3 => invalid[..DOMAIN.len()].fill(0),
            4 => invalid[DOMAIN.len() + 1] = 0,
            5 => invalid[DOMAIN.len() + 2] = b'/',
            6 => {
                let start = DOMAIN.len() + 2 + 10;
                invalid[start..start + 32].fill(0);
            }
            _ => unreachable!(),
        }
        assert!(
            RootEnrolledV3::parse(&invalid).is_err(),
            "malformed root policy variant {variant}",
        );
    }
}

#[test]
fn absence_of_root_authority_never_reads_a_user_provided_policy() {
    if rustix::process::geteuid().as_raw() == 0 {
        return;
    }
    assert!(matches!(
        load_provisioned_root_v3_policy(),
        Err(RootEnrollmentError::NotRoot),
    ));
}

#[test]
fn disposable_root_enrollment_refuses_symlinks_wrong_mode_and_incomplete_marker() {
    if rustix::process::geteuid().as_raw() != 0 {
        // The CI's separately elevated test runs this actual UID0 proof.
        return;
    }
    let temp = tempdir().unwrap();
    let root = temp.path().join("trust");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, Permissions::from_mode(0o700)).unwrap();
    write_root_0600(&root.join("enrollment"), WITNESS_ENROLLMENT);
    write_root_0600(&root.join("b0v3.enrollment"), ENROLLMENT);
    write_root_0600(&root.join("b0v3.policy"), &encoded_policy());
    let dir = File::open(&root).unwrap();
    assert_eq!(
        RootEnrolledV3::from_trust_fd(&dir).unwrap().release_pub,
        [0x44; 32],
    );

    fs::set_permissions(root.join("b0v3.policy"), Permissions::from_mode(0o644)).unwrap();
    assert!(RootEnrolledV3::from_trust_fd(&dir).is_err());
    fs::set_permissions(root.join("b0v3.policy"), Permissions::from_mode(0o600)).unwrap();

    write_root_0600(&root.join("b0v3.enrollment"), b"WRONG-ENROLLMENT\n");
    assert!(RootEnrolledV3::from_trust_fd(&dir).is_err());
    write_root_0600(&root.join("b0v3.enrollment"), ENROLLMENT);

    fs::rename(root.join("b0v3.policy"), root.join("saved")).unwrap();
    symlink("saved", root.join("b0v3.policy")).unwrap();
    assert!(RootEnrolledV3::from_trust_fd(&dir).is_err());
}

#[test]
fn foreign_root_trust_directory_is_always_refused() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    let temp = tempdir().unwrap();
    let root = temp.path().join("trust");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, Permissions::from_mode(0o755)).unwrap();
    let dir = File::open(&root).unwrap();
    assert!(matches!(
        RootEnrolledV3::from_trust_fd(&dir),
        Err(RootEnrollmentError::UntrustedPath),
    ));
}
