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


#[test]
fn disposable_uid0_policy_checks_the_actual_b1a_fd_publisher_and_owner() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    use crate::server_bootstrap_lab::{
        preview_node_v3, B0Error, B0Input, PackagePin, PreviewTransport,
        ReadOnlyHostFacts, ServerTarget,
    };
    use crate::server_installer_pinned_release_lab::{
        verify_root_staged, RootPinnedReleaseTrust,
    };
    use crate::server_owner_approval_wire::OwnerFloorWire;
    use crate::server_release_lab::{
        verify_signed_release, ReleaseFloor, ReleaseManifestV1, SignedRelease,
        TargetPlatform, TrustedReleaseSigner, Version,
    };
    use ring::digest::{digest, SHA256};
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use uuid::Uuid;

    const NOW: u64 = 1_800_000_000;
    struct Kex(Vec<u8>);
    impl PreviewTransport for Kex {
        fn handshake_public_key(
            &mut self, _: &ServerTarget,
        ) -> Result<Vec<u8>, B0Error> {
            Ok(self.0.clone())
        }
        fn inspect_host(&mut self) -> Result<ReadOnlyHostFacts, B0Error> {
            Ok(ReadOnlyHostFacts {
                os: SupportedOs::Ubuntu,
                arch: MachineArch::X86_64,
                has_systemd: true,
                has_sudo_rights: true,
                cpu_count: 2,
                ram_mib: 4096,
                free_disk_mib: 8192,
            })
        }
    }
    let publisher = Ed25519KeyPair::from_seed_unchecked(&[0x61; 32]).unwrap();
    let owner = Ed25519KeyPair::from_seed_unchecked(&[0x62; 32]).unwrap();
    let publisher_public: [u8; 32] = publisher.public_key().as_ref().try_into().unwrap();
    let owner_public: [u8; 32] = owner.public_key().as_ref().try_into().unwrap();
    let node_bytes = b"disposable B1a signed Node V1 fixture, never executed";
    let node_sha: [u8; 32] = digest(&SHA256, node_bytes).as_ref().try_into().unwrap();
    let signer = TrustedReleaseSigner {
        signer_id: "release-test".into(),
        ed25519_public: publisher_public,
    };
    let manifest = ReleaseManifestV1 {
        platform: TargetPlatform::LinuxX86_64,
        version: Version { major: 0, minor: 3, patch: 13 },
        release_sequence: 1,
        package_size: node_bytes.len() as u64,
        package_sha256: node_sha,
        not_before_unix: NOW - 40,
        expires_unix: NOW + 100,
        signer_id: signer.signer_id.clone(),
    };
    let signature: [u8; 64] = publisher.sign(&manifest.canonical_signed_bytes())
        .as_ref().try_into().unwrap();
    let signed = SignedRelease { manifest: manifest.clone(), signature };
    let verified = verify_signed_release(
        &signed, &signer, ReleaseFloor::initial(), NOW,
    ).unwrap();
    let root_release_trust = RootPinnedReleaseTrust {
        trusted_signer: signer,
        previous_floor: ReleaseFloor::initial(),
        approved_manifest_fingerprint:
            verified.release_floor_candidate().signed_manifest_fingerprint,
        approved_package_sha256: node_sha,
        approved_release_sequence: 1,
    };
    let node_dir = tempdir().unwrap();
    fs::set_permissions(node_dir.path(), Permissions::from_mode(0o700)).unwrap();
    for (leaf, bytes) in [
        ("manifest", manifest.canonical_signed_bytes()),
        ("signature", signature.to_vec()),
        ("node", node_bytes.to_vec()),
    ] {
        fs::write(node_dir.path().join(leaf), bytes).unwrap();
        fs::set_permissions(
            node_dir.path().join(leaf), Permissions::from_mode(0o400),
        ).unwrap();
    }
    let fd = verify_root_staged(node_dir.path(), &root_release_trust, NOW)
        .expect("independent B1a signer and package verified from root-owned FD");

    let kex = b"fixed-ci-host-key-no-network".to_vec();
    let host_sha: [u8; 32] = digest(&SHA256, &kex).as_ref().try_into().unwrap();
    let helper_sha = [0x33; 32];
    let input = B0Input {
        install_id: Uuid::from_bytes([0x11; 16]),
        target: ServerTarget {
            host: "example.org".into(), ssh_port: 22, ssh_user: "operator".into(),
            trusted_key_sha256: host_sha,
        },
        package: PackagePin {
            version: "0.3.13".into(),
            package_sha256: node_sha,
            trusted_signer_id: "release-test".into(),
        },
    };
    let plan = preview_node_v3(
        &input, helper_sha, &mut Kex(kex), || false,
    ).unwrap();
    let claim = OwnerApprovalWireV1 {
        approver_id: "owner-test".into(),
        install_id: *input.install_id.as_bytes(),
        plan_digest: *blake3::Hash::from_hex(&plan.plan_blake3).unwrap().as_bytes(),
        ssh_host_sha256: host_sha,
        manifest_fingerprint: fd.receipt.signed_manifest_fingerprint,
        previous: OwnerFloorWire {
            generation: 0, sequence: 0, version: [0; 3],
            manifest_fingerprint: [0; 32], chain_tip: [0; 32],
        },
        proposed: OwnerFloorWire {
            generation: 1, sequence: 1, version: [0, 3, 13],
            manifest_fingerprint: fd.receipt.signed_manifest_fingerprint,
            chain_tip: [0x91; 32],
        },
        staged_package_sha256: node_sha,
        staged_package_size: node_bytes.len() as u64,
        expected_witness_revision: 0,
        approval_nonce: [0xa1; 16],
        issued_at_unix: NOW - 5,
        expires_at_unix: NOW + 45,
    };
    let owner_signature: [u8; 64] =
        owner.sign(&claim.canonical_bytes()).as_ref().try_into().unwrap();
    let mut policy_record = encoded_policy();
    let owner_key_at = DOMAIN.len() + 1 + 1 + "owner-test".len();
    policy_record[owner_key_at..owner_key_at + 32].copy_from_slice(&owner_public);
    policy_record[owner_key_at + 32..owner_key_at + 64].copy_from_slice(&host_sha);
    let publisher_key_at = policy_record.len() - 32;
    policy_record[publisher_key_at..].copy_from_slice(&publisher_public);
    let trusted_dir = tempdir().unwrap();
    fs::set_permissions(trusted_dir.path(), Permissions::from_mode(0o700)).unwrap();
    write_root_0600(&trusted_dir.path().join("enrollment"), WITNESS_ENROLLMENT);
    write_root_0600(&trusted_dir.path().join("b0v3.enrollment"), ENROLLMENT);
    write_root_0600(&trusted_dir.path().join("b0v3.policy"), &policy_record);
    let trust_handle = File::open(trusted_dir.path()).unwrap();
    let actual_policy = RootEnrolledV3::from_trust_fd(&trust_handle).unwrap();
    let raw_plan = serde_json::to_vec(&plan).unwrap();
    let verified_scope = actual_policy.verify_review(
        &raw_plan, &claim, &owner_signature, &fd,
        0, SupportedOs::Ubuntu, MachineArch::X86_64, true, NOW,
    ).unwrap();
    assert_eq!(verified_scope.node_sha256, node_sha);
    assert_eq!(verified_scope.bootstrap_helper_sha256, helper_sha);

    let mut forged_policy = policy_record.clone();
    forged_policy[publisher_key_at] ^= 1;
    let forged_policy = RootEnrolledV3::parse(&forged_policy).unwrap();
    assert!(matches!(
        forged_policy.verify_review(
            &raw_plan, &claim, &owner_signature, &fd,
            0, SupportedOs::Ubuntu, MachineArch::X86_64, true, NOW,
        ),
        Err(RootEnrollmentError::ReleaseIdentityMismatch),
    ));
    let mut forged_owner = owner_signature;
    forged_owner[0] ^= 1;
    assert!(actual_policy.verify_review(
        &raw_plan, &claim, &forged_owner, &fd,
        0, SupportedOs::Ubuntu, MachineArch::X86_64, true, NOW,
    ).is_err());
}
