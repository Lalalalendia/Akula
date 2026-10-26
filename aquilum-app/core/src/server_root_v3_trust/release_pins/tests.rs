use super::*;
use crate::server_release_lab::{
    verify_signed_release, ReleaseManifestV1, SignedRelease, TargetPlatform, Version,
};
use ring::digest::{digest, SHA256};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::fs::{self, Permissions};
use std::os::unix::fs::{symlink, PermissionsExt};
use tempfile::{tempdir, TempDir};

const NOW: u64 = 1_800_000_000;
const NODE_BYTES: &[u8] = b"isolated root-only first Node V1 signed artifact; never execute";

fn write_protected(path: &Path, bytes: &[u8], mode: u32) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, Permissions::from_mode(mode)).unwrap();
}
pub(super) fn fixture() -> (TempDir, FirstReleaseApproval) {
    let folder = tempdir().unwrap();
    let trust = folder.path().join("trust");
    fs::create_dir(&trust).unwrap();
    fs::set_permissions(&trust, Permissions::from_mode(0o700)).unwrap();
    let issuer = Ed25519KeyPair::from_seed_unchecked(&[0x71; 32]).unwrap();
    let issuer_key: [u8; 32] = issuer.public_key().as_ref().try_into().unwrap();
    let node_sha: [u8; 32] = digest(&SHA256, NODE_BYTES).as_ref().try_into().unwrap();
    let mut policy = DOMAIN.to_vec();
    policy.push(3);
    let owner = b"owner-e2e";
    policy.push(owner.len() as u8);
    policy.extend_from_slice(owner);
    policy.extend_from_slice(&[0x21; 32]);
    policy.extend_from_slice(&[0x22; 32]);
    policy.extend_from_slice(&[0x23; 32]);
    let publisher = b"publisher-e2e";
    policy.push(publisher.len() as u8);
    policy.extend_from_slice(publisher);
    policy.extend_from_slice(&issuer_key);
    write_protected(&trust.join("enrollment"), WITNESS_ENROLLMENT, 0o600);
    write_protected(&trust.join("b0v3.enrollment"), ENROLLMENT, 0o600);
    write_protected(&trust.join("b0v3.policy"), &policy, 0o600);

    let manifest = ReleaseManifestV1 {
        platform: TargetPlatform::LinuxX86_64,
        version: Version {
            major: 0,
            minor: 3,
            patch: 13,
        },
        release_sequence: 1,
        package_size: NODE_BYTES.len() as u64,
        package_sha256: node_sha,
        not_before_unix: NOW - 10,
        expires_unix: NOW + 100,
        signer_id: "publisher-e2e".into(),
    };
    let sig: [u8; 64] = issuer
        .sign(&manifest.canonical_signed_bytes())
        .as_ref()
        .try_into()
        .unwrap();
    let verified = verify_signed_release(
        &SignedRelease {
            manifest: manifest.clone(),
            signature: sig,
        },
        &TrustedReleaseSigner {
            signer_id: "publisher-e2e".into(),
            ed25519_public: issuer_key,
        },
        ReleaseFloor::initial(),
        NOW,
    )
    .unwrap();
    let approval = FirstReleaseApproval {
        sequence: 1,
        package_size: NODE_BYTES.len() as u64,
        package_sha256: node_sha,
        manifest_fingerprint: verified
            .release_floor_candidate()
            .signed_manifest_fingerprint,
    };
    write_protected(&trust.join(FIRST_APPROVAL_FILE), &approval.encode(), 0o600);
    write_protected(
        &trust.join(FIRST_APPROVAL_MARKER),
        FIRST_APPROVAL_MARKER_BYTES,
        0o600,
    );
    let stage = trust.join(FIRST_RELEASE_STAGE);
    fs::create_dir(&stage).unwrap();
    fs::set_permissions(&stage, Permissions::from_mode(0o700)).unwrap();
    for (name, bytes) in [
        ("manifest", manifest.canonical_signed_bytes()),
        ("signature", sig.to_vec()),
        ("node", NODE_BYTES.to_vec()),
    ] {
        write_protected(&stage.join(name), &bytes, 0o400);
    }
    (folder, approval)
}
fn check(folder: &TempDir, time: u64) -> Result<RootVerifiedFd, FirstReleaseError> {
    verify_first_from_trust_fd(&File::open(folder.path().join("trust")).unwrap(), time)
}

#[test]
fn fixed_root_public_key_and_signed_package_identity_match() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    let (dir, approved) = fixture();
    let verified = check(&dir, NOW).unwrap();
    assert_eq!(verified.receipt.package_sha256, approved.package_sha256);
    assert_eq!(verified.receipt.byte_length, approved.package_size);
    assert_eq!(verified.receipt.proposed_floor.sequence, 1);
    assert_eq!(verified.verified_publisher().0, "publisher-e2e");
    verified.valid_at(NOW).unwrap();
}

#[test]
fn independent_release_approval_rejects_missing_corrupt_and_downgraded_scope() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    for variant in 0..9 {
        let (dir, mut approval) = fixture();
        let root = dir.path().join("trust");
        match variant {
            0 => fs::remove_file(root.join(FIRST_APPROVAL_MARKER)).unwrap(),
            1 => write_protected(&root.join(FIRST_APPROVAL_MARKER), b"WRONG\n", 0o600),
            2 => {
                approval.package_sha256[0] ^= 1;
                write_protected(&root.join(FIRST_APPROVAL_FILE), &approval.encode(), 0o600);
            }
            3 => {
                approval.manifest_fingerprint[0] ^= 1;
                write_protected(&root.join(FIRST_APPROVAL_FILE), &approval.encode(), 0o600);
            }
            4 => {
                approval.package_size += 1;
                write_protected(&root.join(FIRST_APPROVAL_FILE), &approval.encode(), 0o600);
            }
            5 => {
                approval.sequence += 1;
                write_protected(&root.join(FIRST_APPROVAL_FILE), &approval.encode(), 0o600);
            }
            6 => {
                fs::set_permissions(
                    root.join(FIRST_APPROVAL_FILE),
                    Permissions::from_mode(0o644),
                )
                .unwrap();
            }
            7 => {
                let mut raw = approval.encode();
                raw.push(0);
                write_protected(&root.join(FIRST_APPROVAL_FILE), &raw, 0o600);
            }
            8 => {
                let mut raw = approval.encode();
                raw[APPROVAL_DOMAIN.len()] = 0;
                write_protected(&root.join(FIRST_APPROVAL_FILE), &raw, 0o600);
            }
            _ => unreachable!(),
        }
        assert!(
            check(&dir, NOW).is_err(),
            "bad release approval variant {variant}"
        );
    }
}

#[test]
fn stage_mutations_or_wrong_publisher_and_expiry_are_not_accepted() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    for variant in 0..6 {
        let (dir, _) = fixture();
        let root = dir.path().join("trust");
        let stage = root.join(FIRST_RELEASE_STAGE);
        match variant {
            0 => {
                fs::rename(&stage, root.join("saved-stage")).unwrap();
                symlink("saved-stage", &stage).unwrap();
            }
            1 => write_protected(&stage.join("node"), b"modified artifact", 0o400),
            2 => write_protected(&stage.join("signature"), &[0x99; 64], 0o400),
            3 => {
                let mut policy = fs::read(root.join("b0v3.policy")).unwrap();
                let at = policy.len() - 1;
                policy[at] ^= 1;
                write_protected(&root.join("b0v3.policy"), &policy, 0o600);
            }
            4 => {
                fs::set_permissions(&stage, Permissions::from_mode(0o755)).unwrap();
            }
            5 => {
                fs::remove_file(stage.join("node")).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            check(&dir, NOW).is_err(),
            "bad staged B1a variant {variant}"
        );
    }
    let (dir, _) = fixture();
    assert!(check(&dir, NOW + 100).is_err(), "expired signed release");
}

#[test]
fn without_uid0_even_valid_signed_package_is_inert() {
    if rustix::process::geteuid().as_raw() == 0 {
        return;
    }
    let (dir, _) = fixture();
    assert!(matches!(
        check(&dir, NOW),
        Err(FirstReleaseError::Trust(RootEnrollmentError::NotRoot)),
    ));
}
