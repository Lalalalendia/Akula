use super::*;
use crate::server_release_lab::{ReleaseManifestV1, TargetPlatform, Version};
use crate::server_root_v3_trust::{DOMAIN, ENROLLMENT, WITNESS_ENROLLMENT};
use ring::digest::{digest, SHA256};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::fs::{self, Permissions};
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use tempfile::{tempdir, TempDir};

const NOW: u64 = 1_800_000_000;
const HELPER_SIGNER: &str = "akula-root-bootstrap-v1";
const HOLD_MARKER: &[u8] = b"AKULA_ROOT_BOOTSTRAP_SIGNED_STAGE_ONLY_V1\n";

struct Fixture {
    _root: TempDir,
    trust: File,
    stage: File,
    destination: File,
    helper_sha: [u8; 32],
    bytes: Vec<u8>,
}

fn protected(path: &Path, bytes: &[u8], mode: u32) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, Permissions::from_mode(mode)).unwrap();
}

impl Fixture {
    fn new() -> Self {
        let root = tempdir().unwrap();
        fs::set_permissions(root.path(), Permissions::from_mode(0o700)).unwrap();
        let trust_path = root.path().join("trust");
        let stage_path = root.path().join("bootstrap-v1");
        let destination_path = root.path().join("akula");
        for dir in [&trust_path, &stage_path, &destination_path] {
            fs::create_dir(dir).unwrap();
            fs::set_permissions(dir, Permissions::from_mode(0o700)).unwrap();
        }
        let mut bytes = vec![0x42; 256];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        bytes[18..20].copy_from_slice(b"\x3e\x00");
        let helper_sha: [u8; 32] = digest(&SHA256, &bytes).as_ref().try_into().unwrap();

        let signer = Ed25519KeyPair::from_seed_unchecked(&[0x91; 32]).unwrap();
        let signer_pub: [u8; 32] = signer.public_key().as_ref().try_into().unwrap();
        let manifest = ReleaseManifestV1 {
            platform: TargetPlatform::LinuxX86_64,
            version: Version {
                major: 0,
                minor: 3,
                patch: 13,
            },
            release_sequence: 1,
            package_size: bytes.len() as u64,
            package_sha256: helper_sha,
            not_before_unix: NOW - 30,
            expires_unix: NOW + 60,
            signer_id: HELPER_SIGNER.to_owned(),
        };
        let canonical = manifest.canonical_signed_bytes();
        protected(&stage_path.join("manifest"), &canonical, 0o400);
        protected(
            &stage_path.join("signature"),
            signer.sign(&canonical).as_ref(),
            0o400,
        );
        protected(&stage_path.join("release.ed25519"), &signer_pub, 0o400);
        protected(
            &stage_path.join("release.pub"),
            &bootstrap_helper::expected_pub_pem(&signer_pub),
            0o400,
        );
        protected(&stage_path.join(STAGED_LEAF), &bytes, 0o400);
        let mut hold_marker = HOLD_MARKER.to_vec();
        // The canonical marker is lowercase hex without delimiters.
        for b in helper_sha {
            hold_marker.extend_from_slice(format!("{b:02x}").as_bytes());
        }
        hold_marker.push(b'\n');
        protected(&stage_path.join("HOLD"), &hold_marker, 0o400);

        let mut policy = DOMAIN.to_vec();
        policy.push(3);
        let owner = b"owner-fixture";
        policy.push(owner.len() as u8);
        policy.extend_from_slice(owner);
        policy.extend_from_slice(&[0x11; 32]);
        policy.extend_from_slice(&[0x22; 32]);
        policy.extend_from_slice(&helper_sha);
        let publisher = b"publisher-fixture";
        policy.push(publisher.len() as u8);
        policy.extend_from_slice(publisher);
        policy.extend_from_slice(&[0x44; 32]);
        protected(&trust_path.join("enrollment"), WITNESS_ENROLLMENT, 0o600);
        protected(&trust_path.join("b0v3.enrollment"), ENROLLMENT, 0o600);
        protected(&trust_path.join("b0v3.policy"), &policy, 0o600);

        Self {
            trust: File::open(&trust_path).unwrap(),
            stage: File::open(&stage_path).unwrap(),
            destination: File::open(&destination_path).unwrap(),
            helper_sha,
            bytes,
            _root: root,
        }
    }
    fn approved(&self) -> bootstrap_helper::BootstrapHelperHold {
        bootstrap_helper::inspect_from_trust_fd(&self.trust, NOW).unwrap()
    }
    fn dest_path(&self) -> std::path::PathBuf {
        self._root.path().join("akula")
    }
    fn stage_path(&self) -> std::path::PathBuf {
        self._root.path().join("bootstrap-v1")
    }
}

#[test]
fn nonroot_rejects_root_helper_publication() {
    if rustix::process::geteuid().as_raw() == 0 {
        return;
    }
    let f = Fixture::new();
    let forged = bootstrap_helper::BootstrapHelperHold {
        sha256: f.helper_sha,
        size: f.bytes.len() as u64,
        release_sequence: 1,
        remains_hold: true,
    };
    assert!(matches!(
        publish_signed_hold(&f.stage, &f.destination, &forged),
        Err(HelperInstallError::NotRoot)
    ));
    assert_eq!(fs::read_dir(f.dest_path()).unwrap().count(), 0);
    assert!(matches!(
        admin_install_fixed_root_helper_once(),
        Err(HelperInstallError::NotRoot)
    ));
}

#[test]
fn root_signed_stage_publishes_exact_0500_copy_and_never_executes() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    let f = Fixture::new();
    let verified = f.approved();
    let published = publish_signed_hold(&f.stage, &f.destination, &verified).unwrap();
    assert_eq!(published.sha256, f.helper_sha);
    assert_eq!(published.size, f.bytes.len() as u64);
    assert!(published.remains_hold);
    let installed = f.dest_path().join(EXECUTABLE_LEAF);
    assert_eq!(fs::read(&installed).unwrap(), f.bytes);
    let meta = fs::metadata(&installed).unwrap();
    assert_eq!(meta.uid(), 0);
    assert_eq!(meta.nlink(), 1);
    assert_eq!(meta.permissions().mode() & 0o7777, 0o500);
    assert_ne!(
        meta.ino(),
        fs::metadata(f.stage_path().join(STAGED_LEAF)).unwrap().ino()
    );
    assert_eq!(fs::read_dir(f.dest_path()).unwrap().count(), 1);
    assert!(!f.dest_path().join(PENDING_LEAF).exists());
    assert!(matches!(
        publish_signed_hold(&f.stage, &f.destination, &verified),
        Err(HelperInstallError::ExistingOrInterrupted)
    ));
    assert_eq!(fs::read(&installed).unwrap(), f.bytes);
}

#[test]
fn root_rejects_tamper_collision_symlink_and_untrusted_inode_without_cleanup() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    for case in 0..12 {
        let f = Fixture::new();
        let mut verified = f.approved();
        let source = f.stage_path().join(STAGED_LEAF);
        let final_path = f.dest_path().join(EXECUTABLE_LEAF);
        let pending_path = f.dest_path().join(PENDING_LEAF);
        match case {
            0 => verified.sha256[0] ^= 1,
            1 => verified.release_sequence = 0,
            2 => fs::set_permissions(&source, Permissions::from_mode(0o600)).unwrap(),
            3 => {
                fs::remove_file(&source).unwrap();
                symlink("helper-original", &source).unwrap();
            }
            4 => fs::hard_link(&source, f.stage_path().join("hardlink")).unwrap(),
            5 => {
                fs::set_permissions(&source, Permissions::from_mode(0o600)).unwrap();
                fs::write(&source, b"not-an-elf").unwrap();
                fs::set_permissions(&source, Permissions::from_mode(0o400)).unwrap();
            }
            6 => {
                fs::set_permissions(f.stage_path(), Permissions::from_mode(0o777)).unwrap();
            }
            7 => {
                fs::set_permissions(f.dest_path(), Permissions::from_mode(0o777)).unwrap();
            }
            8 => fs::write(&final_path, b"existing").unwrap(),
            9 => fs::write(&pending_path, b"incomplete").unwrap(),
            10 => verified.size += 1,
            11 => verified.remains_hold = false,
            _ => unreachable!(),
        }
        assert!(
            publish_signed_hold(&f.stage, &f.destination, &verified).is_err(),
            "unsafe variant {case} became a completed installation"
        );
        if case == 8 {
            assert_eq!(fs::read(&final_path).unwrap(), b"existing");
        } else {
            assert!(!final_path.exists(), "variant {case} published helper");
        }
        if case == 9 {
            assert_eq!(fs::read(&pending_path).unwrap(), b"incomplete");
        }
    }
}

#[test]
fn exact_reviewed_executable_path_is_not_caller_selected() {
    assert_eq!(
        crate::server_bootstrap_lab::ROOT_HELPER_EXECUTABLE_PATH,
        "/usr/libexec/akula/akula-root-b1d0-helper"
    );
}
