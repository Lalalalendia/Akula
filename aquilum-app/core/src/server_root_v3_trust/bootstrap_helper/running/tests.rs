use super::*;
use ring::digest::{digest, SHA256};
use std::fs::{self, Permissions};
use std::os::unix::fs::{symlink, PermissionsExt};
use tempfile::{tempdir, TempDir};

fn fixture() -> (TempDir, File, File, BootstrapHelperHold) {
    let temp = tempdir().unwrap();
    fs::set_permissions(temp.path(), Permissions::from_mode(0o700)).unwrap();
    let path = temp.path().join(EXEC_LEAF);
    let mut bytes = vec![0x42; 256];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    bytes[18..20].copy_from_slice(b"\x3e\x00");
    fs::write(&path, &bytes).unwrap();
    fs::set_permissions(&path, Permissions::from_mode(EXEC_MODE)).unwrap();
    let sha256 = digest(&SHA256, &bytes).as_ref().try_into().unwrap();
    let hold = BootstrapHelperHold {
        sha256,
        size: bytes.len() as u64,
        release_sequence: 1,
        remains_hold: true,
    };
    (temp, File::open(&path).unwrap(), File::open(path).unwrap(), hold)
}

#[test]
fn running_process_inode_must_be_the_same_root_owned_signed_executable() {
    let (temp, fixed, running, hold) = fixture();
    if rustix::process::geteuid().as_raw() != 0 {
        assert!(verify_identical_images(&fixed, &running, &hold).is_err());
        return;
    }
    verify_identical_images(&fixed, &running, &hold).unwrap();
    let other_path = temp.path().join("same-signed-bytes-other-inode");
    fs::copy(temp.path().join(EXEC_LEAF), &other_path).unwrap();
    fs::set_permissions(&other_path, Permissions::from_mode(EXEC_MODE)).unwrap();
    let unrelated = File::open(&other_path).unwrap();
    // Hash equality alone is insufficient: the process must really run the
    // exact fixed root-owned, previously signed executable inode.
    assert!(verify_identical_images(&fixed, &unrelated, &hold).is_err());
}

#[test]
fn changed_digest_missing_hold_unsafe_mode_hardlink_and_truncation_refuse() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    for variant in 0..6 {
        let (temp, fixed, running, hold) = fixture();
        let path = temp.path().join(EXEC_LEAF);
        let mut expected = hold;
        match variant {
            0 => expected.sha256[0] ^= 1,
            1 => expected.remains_hold = false,
            2 => fs::set_permissions(&path, Permissions::from_mode(0o700)).unwrap(),
            3 => fs::hard_link(&path, temp.path().join("duplicate-link")).unwrap(),
            4 => {
                fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
                fs::write(&path, b"\x7fELFshort").unwrap();
                fs::set_permissions(&path, Permissions::from_mode(EXEC_MODE)).unwrap();
            }
            5 => expected.size += 1,
            _ => unreachable!(),
        }
        assert!(
            verify_identical_images(&fixed, &running, &expected).is_err(),
            "unsafe running image variant {variant} admitted"
        );
    }
}

#[test]
fn fixed_directory_and_executable_symlinks_never_become_authority() {
    let (temp, fixed, _, _) = fixture();
    let dir = File::open(temp.path()).unwrap();
    let unsafe_dir = verified_dir(&dir, 0o755);
    assert!(unsafe_dir.is_err());
    // Linux O_NOFOLLOW prevents a peer-selected symlink to a different root
    // binary, including an otherwise byte-identical copy.
    symlink(EXEC_LEAF, temp.path().join("alias")).unwrap();
    assert!(
        openat(
            &dir,
            "alias",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .is_err()
    );
    let meta = fixed.metadata().unwrap();
    if rustix::process::geteuid().as_raw() == 0 {
        assert_eq!(meta.uid(), 0);
    }
}
