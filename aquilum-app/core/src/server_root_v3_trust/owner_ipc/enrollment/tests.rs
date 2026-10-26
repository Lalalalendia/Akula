use super::*;
use std::fs::{self, Permissions};
use std::os::unix::fs::{symlink, PermissionsExt};
use tempfile::{tempdir, TempDir};

const TEST_UID: u32 = 990;

fn candidate(uid: u32) -> Vec<u8> {
    let mut bytes = PEER_DOMAIN.to_vec();
    bytes.extend_from_slice(&uid.to_be_bytes());
    bytes
}
fn protected(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, Permissions::from_mode(0o600)).unwrap();
}
fn fixture() -> (TempDir, File) {
    let temp = tempdir().unwrap();
    let dir = temp.path().join("trust");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, Permissions::from_mode(0o700)).unwrap();
    protected(&dir.join("enrollment"), WITNESS_ENROLLMENT);
    protected(&dir.join("b0v3.enrollment"), ENROLLMENT);
    let mut policy = DOMAIN.to_vec();
    policy.push(3);
    let owner = b"owner-ipc";
    policy.push(owner.len() as u8);
    policy.extend_from_slice(owner);
    policy.extend_from_slice(&[0x11; 32]);
    policy.extend_from_slice(&[0x22; 32]);
    policy.extend_from_slice(&[0x33; 32]);
    let publisher = b"publisher-ipc";
    policy.push(publisher.len() as u8);
    policy.extend_from_slice(publisher);
    policy.extend_from_slice(&[0x44; 32]);
    protected(&dir.join("b0v3.policy"), &policy);
    (temp, File::open(dir).unwrap())
}
fn no_pin(dir: &TempDir) -> bool {
    !dir.path().join("trust").join(PEER_PIN_FILE).exists()
}

#[test]
fn one_use_peer_uid_registration_is_root_only_and_never_creates_install_authority() {
    let (temp, trust) = fixture();
    let bytes = candidate(TEST_UID);
    if rustix::process::geteuid().as_raw() != 0 {
        assert!(matches!(
            enroll_once_from_admin_bytes(&trust, &bytes),
            Err(PeerEnrollError::Trust(RootEnrollmentError::NotRoot))
        ));
        assert!(no_pin(&temp));
        return;
    }
    enroll_once_from_admin_bytes(&trust, &bytes).unwrap();
    assert_eq!(read_allowed_peer(&trust).unwrap(), TEST_UID);
    let root = temp.path().join("trust");
    assert_eq!(fs::read(root.join(PEER_PIN_FILE)).unwrap(), bytes);
    assert_eq!(
        fs::metadata(root.join(PEER_PIN_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o600
    );
    assert!(!root.join(PEER_PENDING).exists());
    assert!(matches!(
        enroll_once_from_admin_bytes(&trust, &bytes),
        Err(PeerEnrollError::AlreadyEnrolledOrInterrupted)
    ));
    // No root helper executable, Node install, service or CAS was produced.
    assert_eq!(fs::read_dir(&root).unwrap().count(), 4);
}

#[test]
fn invalid_candidates_missing_policy_and_interrupted_registration_hold() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    for variant in 0..12 {
        let (temp, trust) = fixture();
        let dir = temp.path().join("trust");
        let mut bytes = candidate(TEST_UID);
        match variant {
            0 => bytes[0] ^= 1,
            1 => bytes.push(0),
            2 => {
                bytes.pop();
            }
            3 => bytes = candidate(0),
            4 => bytes = candidate(65534),
            5 => bytes = candidate(u32::MAX),
            6 => {
                protected(&dir.join("b0v3.enrollment"), b"WRONG\n");
            }
            7 => {
                fs::remove_file(dir.join("b0v3.policy")).unwrap();
            }
            8 => {
                protected(&dir.join(PEER_PENDING), b"interrupted");
            }
            9 => {
                protected(&dir.join(PEER_PIN_FILE), b"other");
            }
            10 => {
                symlink("missing-peer", dir.join(PEER_PIN_FILE)).unwrap();
            }
            11 => {
                fs::set_permissions(&dir, Permissions::from_mode(0o755)).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            enroll_once_from_admin_bytes(&trust, &bytes).is_err(),
            "variant {variant} must hold"
        );
        if variant != 9 && variant != 10 {
            assert!(no_pin(&temp), "unexpected pin variant {variant}");
        }
    }
}

#[test]
fn fixed_local_nonlogin_account_parser_refuses_uid_aliasing_and_unsafe_shell() {
    let header = b"root:x:0:0:root:/root:/bin/bash\nnobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin\n";
    let account = b"akula-provision:x:990:991:Akula:/nonexistent:/usr/sbin/nologin\n";
    let mut valid = header.to_vec();
    valid.extend_from_slice(account);
    assert!(parse_local_account(&valid, TEST_UID).is_ok());
    for invalid in [
        b"akula-provision:x:991:991:Akula:/nonexistent:/usr/sbin/nologin\n".as_slice(),
        b"akula-provision:x:990:991:Akula:/home/akula:/usr/sbin/nologin\n",
        b"akula-provision:x:990:991:Akula:/nonexistent:/bin/bash\n",
        b"akula-provision:x:990:0:Akula:/nonexistent:/usr/sbin/nologin\n",
        b"akula-provision:*:990:991:Akula:/nonexistent:/usr/sbin/nologin\n",
        b"other:x:990:991:Other:/nonexistent:/usr/sbin/nologin\n",
    ] {
        let mut bytes = header.to_vec();
        bytes.extend_from_slice(invalid);
        assert!(parse_local_account(&bytes, TEST_UID).is_err());
    }
    let mut alias = valid.clone();
    alias.extend_from_slice(b"another:x:990:990:Other:/nonexistent:/usr/sbin/nologin\n");
    assert!(parse_local_account(&alias, TEST_UID).is_err());
    let mut duplicate = valid.clone();
    duplicate.extend_from_slice(account);
    assert!(parse_local_account(&duplicate, TEST_UID).is_err());
    let mut corrupted = valid.clone();
    corrupted.push(0xff);
    assert!(parse_local_account(&corrupted, TEST_UID).is_err());
    assert!(parse_local_account(&valid, 65534).is_err());
}
