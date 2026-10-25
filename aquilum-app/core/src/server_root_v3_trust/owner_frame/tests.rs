use super::*;
use crate::server_owner_approval_wire::{OwnerApprovalWireV1, OwnerFloorWire};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::fs::{self, Permissions};
use std::os::unix::fs::{symlink, PermissionsExt};
use tempfile::tempdir;

const NOW: u64 = 1_800_000_000;
const OWNER_NAME: &str = "owner-e2e-frame";

fn policy(key: &[u8; 32]) -> Vec<u8> {
    let mut data = DOMAIN.to_vec();
    data.push(3);
    data.push(OWNER_NAME.len() as u8);
    data.extend_from_slice(OWNER_NAME.as_bytes());
    data.extend_from_slice(key);
    data.extend_from_slice(&[0x22; 32]); // independently pinned SSH host
    data.extend_from_slice(&[0x33; 32]); // independently pinned helper
    let publisher = b"publisher-e2e";
    data.push(publisher.len() as u8);
    data.extend_from_slice(publisher);
    data.extend_from_slice(&[0x44; 32]);
    data
}
fn write(path: &Path, data: &[u8]) {
    fs::write(path, data).unwrap();
    fs::set_permissions(path, Permissions::from_mode(0o600)).unwrap();
}
fn fixture(public: &[u8; 32]) -> (tempfile::TempDir, File) {
    let temp = tempdir().unwrap();
    let trust = temp.path().join("trust");
    fs::create_dir(&trust).unwrap();
    fs::set_permissions(&trust, Permissions::from_mode(0o700)).unwrap();
    write(&trust.join("enrollment"), WITNESS_ENROLLMENT);
    write(&trust.join("b0v3.enrollment"), ENROLLMENT);
    write(&trust.join("b0v3.policy"), &policy(public));
    (temp, File::open(&trust).unwrap())
}
fn frame(signer: &Ed25519KeyPair) -> Vec<u8> {
    let claim = OwnerApprovalWireV1 {
        approver_id: OWNER_NAME.into(),
        install_id: [0x11; 16],
        plan_digest: [0x22; 32],
        ssh_host_sha256: [0x23; 32],
        manifest_fingerprint: [0x24; 32],
        previous: OwnerFloorWire {
            generation: 0,
            sequence: 0,
            version: [0; 3],
            manifest_fingerprint: [0; 32],
            chain_tip: [0; 32],
        },
        proposed: OwnerFloorWire {
            generation: 1,
            sequence: 1,
            version: [0, 3, 13],
            manifest_fingerprint: [0x24; 32],
            chain_tip: [0x25; 32],
        },
        staged_package_sha256: [0x26; 32],
        staged_package_size: 4096,
        expected_witness_revision: 0,
        approval_nonce: [0x27; 16],
        issued_at_unix: NOW - 5,
        expires_at_unix: NOW + 50,
    };
    let mut result = claim.canonical_bytes();
    let signature = signer.sign(&result);
    result.extend_from_slice(signature.as_ref());
    result
}

#[test]
fn disposable_uid0_accepts_exact_owner_frame_only_from_enrolled_policy() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    let signer = Ed25519KeyPair::from_seed_unchecked(&[0x80; 32]).unwrap();
    let public: [u8; 32] = signer.public_key().as_ref().try_into().unwrap();
    let (_temp, trust) = fixture(&public);
    let signed = frame(&signer);
    let accepted = verify_from_trust_fd(&trust, &signed, NOW).unwrap();
    assert_eq!(accepted.claim.canonical_bytes(), signed[..signed.len() - 64]);
    assert_eq!(accepted.signature.as_slice(), &signed[signed.len() - 64..]);
    // This read-only check creates no B1d0 CAS or Node file.
    assert!(verify_from_trust_fd(&trust, &signed, NOW + 50).is_err());
}

#[test]
fn wrong_key_or_missing_root_marker_fails_before_any_authority() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    let signer = Ed25519KeyPair::from_seed_unchecked(&[0x81; 32]).unwrap();
    let other = Ed25519KeyPair::from_seed_unchecked(&[0x82; 32]).unwrap();
    let other_public: [u8; 32] = other.public_key().as_ref().try_into().unwrap();
    let (temp, trust) = fixture(&other_public);
    let signed = frame(&signer);
    assert!(matches!(
        verify_from_trust_fd(&trust, &signed, NOW),
        Err(RootEnrollmentError::OwnerFrame(WireError::BadSignature))
    ));
    write(
        &temp.path().join("trust").join("b0v3.enrollment"),
        b"WRONG\n",
    );
    assert!(verify_from_trust_fd(&trust, &signed, NOW).is_err());
}

#[test]
fn forged_or_incomplete_wire_and_symlinked_policy_are_refused() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    let signer = Ed25519KeyPair::from_seed_unchecked(&[0x83; 32]).unwrap();
    let public: [u8; 32] = signer.public_key().as_ref().try_into().unwrap();
    let (temp, trust) = fixture(&public);
    let signed = frame(&signer);
    let mut changed = signed.clone();
    changed[DOMAIN.len() + 7] ^= 1;
    assert!(verify_from_trust_fd(&trust, &changed, NOW).is_err());
    assert!(verify_from_trust_fd(&trust, &signed[..signed.len() - 1], NOW).is_err());
    let mut appended = signed.clone();
    appended.push(0);
    assert!(verify_from_trust_fd(&trust, &appended, NOW).is_err());
    let path = temp.path().join("trust");
    fs::rename(path.join("b0v3.policy"), path.join("saved-policy")).unwrap();
    symlink("saved-policy", path.join("b0v3.policy")).unwrap();
    assert!(verify_from_trust_fd(&trust, &signed, NOW).is_err());
}

#[test]
fn unprivileged_process_cannot_approve_a_frame_even_with_correct_signature() {
    if rustix::process::geteuid().as_raw() == 0 {
        return;
    }
    let signer = Ed25519KeyPair::from_seed_unchecked(&[0x84; 32]).unwrap();
    let public: [u8; 32] = signer.public_key().as_ref().try_into().unwrap();
    let (_temp, trust) = fixture(&public);
    assert!(matches!(
        verify_from_trust_fd(&trust, &frame(&signer), NOW),
        Err(RootEnrollmentError::NotRoot)
    ));
}
