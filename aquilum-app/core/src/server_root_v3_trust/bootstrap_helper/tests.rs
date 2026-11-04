use super::*;
use crate::server_release_lab::{ReleaseManifestV1, TargetPlatform, Version};
use ring::digest::{digest, SHA256};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::fs::{self, Permissions};
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use tempfile::{tempdir, TempDir};

const NOW: u64 = 1_800_000_000;

fn write_protected(path: &Path, bytes: &[u8], mode: u32) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, Permissions::from_mode(mode)).unwrap();
}
fn fake_elf() -> Vec<u8> {
    let mut data = vec![0x42; 256];
    data[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    data[18..20].copy_from_slice(b"\x3e\x00");
    data
}
fn marker(sha: &[u8; 32]) -> Vec<u8> {
    let mut data = HOLD_DOMAIN.to_vec();
    for b in sha {
        data.extend_from_slice(format!("{b:02x}").as_bytes());
    }
    data.push(b'\n');
    data
}
fn fixture() -> (TempDir, File, [u8; 32]) {
    let temp = tempdir().unwrap();
    // The trust parent in production is a fixed UID0 0700 directory.
    // Tempfile implementations need not inherit the same permission mode.
    fs::set_permissions(temp.path(), Permissions::from_mode(0o700)).unwrap();
    let trust_path = temp.path().join("trust");
    let stage_path = temp.path().join(STAGE_LEAF);
    fs::create_dir(&trust_path).unwrap();
    fs::create_dir(&stage_path).unwrap();
    for dir in [&trust_path, &stage_path] {
        fs::set_permissions(dir, Permissions::from_mode(0o700)).unwrap();
    }
    let package = fake_elf();
    let sha: [u8; 32] = digest(&SHA256, &package).as_ref().try_into().unwrap();

    // B0v3 was enrolled separately from the signed bootstrap stage.
    let mut policy = DOMAIN.to_vec();
    policy.push(3);
    let owner = b"owner-test";
    policy.push(owner.len() as u8);
    policy.extend_from_slice(owner);
    policy.extend_from_slice(&[0x11; 32]);
    policy.extend_from_slice(&[0x22; 32]);
    policy.extend_from_slice(&sha);
    let publisher = b"release-test";
    policy.push(publisher.len() as u8);
    policy.extend_from_slice(publisher);
    policy.extend_from_slice(&[0x44; 32]);
    write_protected(&trust_path.join("enrollment"), WITNESS_ENROLLMENT, 0o600);
    write_protected(&trust_path.join("b0v3.enrollment"), ENROLLMENT, 0o600);
    write_protected(&trust_path.join("b0v3.policy"), &policy, 0o600);

    let signer = Ed25519KeyPair::from_seed_unchecked(&[0x91; 32]).unwrap();
    let public: [u8; 32] = signer.public_key().as_ref().try_into().unwrap();
    let manifest = ReleaseManifestV1 {
        platform: TargetPlatform::LinuxX86_64,
        version: Version {
            major: 0,
            minor: 3,
            patch: 13,
        },
        release_sequence: 1,
        package_size: package.len() as u64,
        package_sha256: sha,
        not_before_unix: NOW - 30,
        expires_unix: NOW + 60,
        signer_id: HELPER_SIGNER.to_owned(),
    };
    let signed = manifest.canonical_signed_bytes();
    write_protected(&stage_path.join("manifest"), &signed, 0o400);
    write_protected(
        &stage_path.join("signature"),
        signer.sign(&signed).as_ref(),
        0o400,
    );
    write_protected(&stage_path.join("release.ed25519"), &public, 0o400);
    write_protected(
        &stage_path.join("release.pub"),
        &expected_pub_pem(&public),
        0o400,
    );
    write_protected(&stage_path.join(HELPER_LEAF), &package, 0o400);
    write_protected(&stage_path.join("HOLD"), &marker(&sha), 0o400);
    let trust = File::open(trust_path).unwrap();
    (temp, trust, sha)
}

#[test]
fn linux_bootstrap_pem_matches_independent_openssl_spki_test_vector() {
    let expected = b"-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=\n-----END PUBLIC KEY-----\n";
    assert_eq!(expected_pub_pem(&[1; 32]), expected);
}

#[test]
fn independently_pinned_root_policy_accepts_only_signed_stage_hold() {
    let (temp, trust, sha) = fixture();
    if rustix::process::geteuid().as_raw() != 0 {
        assert!(matches!(
            inspect_from_trust_fd(&trust, NOW),
            Err(BootstrapHelperError::Trust(RootEnrollmentError::NotRoot))
        ));
        return;
    }
    // Verify each independent fixture boundary before the composite check,
    // so a regression points to a precise signed/OS-owned contract.
    let policy = RootEnrolledV3::from_trust_fd(&trust).unwrap();
    assert_eq!(policy.helper_sha256, sha, "root-enrolled helper digest");
    let parent = File::from(
        openat(
            &trust,
            "..",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap(),
    );
    let parent_meta = parent.metadata().unwrap();
    assert!(
        root_dir_ok(&parent_meta, 0o700),
        "parent root uid={}, mode={:o}",
        parent_meta.uid(),
        parent_meta.permissions().mode() & 0o7777
    );
    let stage = File::from(
        openat(
            &parent,
            STAGE_LEAF,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap(),
    );
    assert!(root_dir_ok(&stage.metadata().unwrap(), 0o700), "stage root");
    let manifest = decode_b1a_wire(&protected(&stage, "manifest", 256).unwrap()).unwrap();
    assert_eq!(manifest.signer_id, HELPER_SIGNER, "signed helper identity");
    assert_eq!(manifest.package_sha256, sha, "manifest SHA");
    let key: [u8; 32] = protected(&stage, "release.ed25519", 32)
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(
        protected(&stage, "release.pub", 1024).unwrap(),
        expected_pub_pem(&key),
        "PEM matches exact signer"
    );
    assert_eq!(
        protected(&stage, "HOLD", 256).unwrap(),
        marker(&sha),
        "durable HOLD marker"
    );
    let bytes = protected(&stage, HELPER_LEAF, MAX_HELPER_BYTES).unwrap();
    assert_eq!(bytes.len(), manifest.package_size as usize, "package size");
    assert_eq!(&bytes[..7], b"\x7fELF\x02\x01\x01", "ELF magic");
    assert_eq!(&bytes[18..20], b"\x3e\x00", "x86-64");
    assert_eq!(digest(&SHA256, &bytes).as_ref(), sha, "ELF digest");
    let result = inspect_from_trust_fd(&trust, NOW).unwrap();
    assert_eq!(
        result,
        BootstrapHelperHold {
            sha256: sha,
            size: 256,
            release_sequence: 1,
            remains_hold: true,
        }
    );
    assert!(temp.path().join(STAGE_LEAF).join("HOLD").is_file());
    assert!(!temp.path().join(STAGE_LEAF).join("helper").exists());
    assert!(inspect_from_trust_fd(&trust, NOW + 60).is_err());
}

#[test]
fn signed_but_wrong_helper_identity_tampered_stage_or_unsafe_inode_holds() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    for variant in 0..12 {
        let (temp, trust, _) = fixture();
        let stage = temp.path().join(STAGE_LEAF);
        let trust_dir = temp.path().join("trust");
        match variant {
            0 => {
                let mut pkg = fs::read(stage.join(HELPER_LEAF)).unwrap();
                pkg[30] ^= 1;
                fs::write(stage.join(HELPER_LEAF), pkg).unwrap();
            }
            1 => {
                fs::write(stage.join("HOLD"), b"faked success\n").unwrap();
            }
            2 => {
                let mut sig = fs::read(stage.join("signature")).unwrap();
                sig[0] ^= 1;
                fs::write(stage.join("signature"), sig).unwrap();
            }
            3 => {
                fs::write(stage.join("release.ed25519"), [0x23; 32]).unwrap();
            }
            4 => {
                let policy_file = trust_dir.join("b0v3.policy");
                let mut policy = fs::read(&policy_file).unwrap();
                let offset = DOMAIN.len() + 2 + b"owner-test".len() + 64;
                policy[offset] ^= 1;
                fs::write(&policy_file, policy).unwrap();
            }
            5 => {
                fs::rename(stage.join(HELPER_LEAF), stage.join("saved-helper")).unwrap();
                symlink("saved-helper", stage.join(HELPER_LEAF)).unwrap();
            }
            6 => {
                fs::hard_link(stage.join(HELPER_LEAF), stage.join("other-helper")).unwrap();
            }
            7 => {
                fs::rename(&stage, temp.path().join("saved-stage")).unwrap();
                symlink("saved-stage", &stage).unwrap();
            }
            8 => {
                fs::set_permissions(stage.join(HELPER_LEAF), Permissions::from_mode(0o444))
                    .unwrap();
            }
            9 => {
                fs::write(stage.join("release.pub"), b"different PEM").unwrap();
            }
            10 => {
                fs::write(stage.join(HELPER_LEAF), b"\x7fELFshort").unwrap();
            }
            11 => {
                fs::remove_file(stage.join("manifest")).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            inspect_from_trust_fd(&trust, NOW).is_err(),
            "stage variant {variant} must HOLD"
        );
    }
}
