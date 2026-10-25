use super::*;
use std::fs::{self, Permissions};
use std::os::unix::fs::{symlink, PermissionsExt};
use tempfile::tempdir;

fn opt_shaped_parent(root: &Path) -> File {
    let opt = root.join("opt");
    fs::create_dir(&opt).unwrap();
    fs::set_permissions(&opt, Permissions::from_mode(0o755)).unwrap();
    File::open(opt).unwrap()
}

#[test]
fn nonroot_never_creates_install_destination() {
    if rustix::process::geteuid().as_raw() == 0 { return; }
    let tmp = tempdir().unwrap();
    let opt = opt_shaped_parent(tmp.path());
    assert!(matches!(
        first_opt_destination(&opt),
        Err(OptFirstEnrollError::NotRoot),
    ));
    assert!(!tmp.path().join("opt/akula-node").exists());
}

#[test]
fn uid0_empty_destination_is_single_use_and_exactly_b1d0_enrolled() {
    if rustix::process::geteuid().as_raw() != 0 { return; }
    let tmp = tempdir().unwrap();
    let opt = opt_shaped_parent(tmp.path());
    first_opt_destination(&opt).unwrap();
    let target = tmp.path().join("opt/akula-node");
    assert!(root_dir_ok(&fs::metadata(&target).unwrap(), 0o755));
    assert!(root_file_ok(&fs::metadata(target.join(OPT_MARKER)).unwrap(), 0o600, 128));
    assert_eq!(fs::read(target.join(OPT_MARKER)).unwrap(), OPT_ENROLLMENT);
    assert!(!target.join(OPT_MARKER_PENDING).exists());
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    // Replaying the administrator CLI must not recreate, erase or replace
    // the destination marker or any existing Node binary.
    assert!(matches!(
        first_opt_destination(&opt),
        Err(OptFirstEnrollError::ExistingOrInterrupted),
    ));
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
}

#[test]
fn collision_partial_directory_and_symlink_are_nonrepairable_hold() {
    if rustix::process::geteuid().as_raw() != 0 { return; }
    for variant in 0..3 {
        let tmp = tempdir().unwrap();
        let opt = opt_shaped_parent(tmp.path());
        let destination = tmp.path().join("opt/akula-node");
        match variant {
            0 => {
                fs::create_dir(&destination).unwrap();
                fs::write(destination.join("node.pending"), b"unverified").unwrap();
            }
            1 => {
                fs::create_dir(&destination).unwrap();
                fs::write(destination.join(OPT_MARKER_PENDING), b"interrupted").unwrap();
            }
            2 => {
                let outside = tmp.path().join("outside");
                fs::create_dir(&outside).unwrap();
                symlink(&outside, &destination).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(matches!(
            first_opt_destination(&opt),
            Err(OptFirstEnrollError::ExistingOrInterrupted),
        ));
        assert!(!destination.join(OPT_MARKER).exists());
    }
}

#[test]
fn world_writable_opt_parent_is_rejected_before_mkdir() {
    if rustix::process::geteuid().as_raw() != 0 { return; }
    let tmp = tempdir().unwrap();
    let opt = opt_shaped_parent(tmp.path());
    fs::set_permissions(tmp.path().join("opt"), Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(
        first_opt_destination(&opt),
        Err(OptFirstEnrollError::ParentUntrusted),
    ));
    assert!(!tmp.path().join("opt/akula-node").exists());
}
