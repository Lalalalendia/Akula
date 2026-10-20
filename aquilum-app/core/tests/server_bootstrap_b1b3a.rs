//! Linux AF_UNIX integration tests for signed, side-effect-free B1b3a
//! installation preflight. All signing seeds are public synthetic TEST data.
#![cfg(all(target_os = "linux", feature = "server-bootstrap-b1b3a-helper-ipc-lab"))]

use aquilum_core::server_installer_ipc_lab::{
    admit_preflight, InertPreflightResult, PreflightClaim, PreflightError,
    RootTrustedPolicy, SignedPreflight, MAX_VALIDITY_SECONDS, WIRE_BYTES,
};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::io::Write;
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::thread;

const NOW: u64 = 1_800_000_000;
fn key() -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[0x42; 32]).unwrap()
}
fn anchor() -> RootTrustedPolicy {
    RootTrustedPolicy {
        service_euid: rustix::process::geteuid().as_raw(),
        allowed_peer_uid: rustix::process::geteuid().as_raw(),
        pinned_approver_ed25519: key().public_key().as_ref().try_into().unwrap(),
        install_id: [0x11; 16],
        approved_plan_digest: [0x22; 32],
        pinned_ssh_host_sha256: [0x33; 32],
        signed_manifest_fingerprint: [0x44; 32],
        staged_package_sha256: [0x55; 32],
        expected_release_sequence: 37,
        expected_witness_revision: 7,
    }
}
fn claim() -> PreflightClaim {
    let p = anchor();
    PreflightClaim {
        install_id: p.install_id,
        plan_digest: p.approved_plan_digest,
        ssh_host_sha256: p.pinned_ssh_host_sha256,
        signed_manifest_fingerprint: p.signed_manifest_fingerprint,
        package_sha256: p.staged_package_sha256,
        release_sequence: p.expected_release_sequence,
        witness_revision: p.expected_witness_revision,
        approval_nonce: [0x77; 16],
        issued_at_unix: NOW - 60,
        expires_at_unix: NOW + 120,
    }
}
fn signed(value: PreflightClaim) -> SignedPreflight {
    let k = key();
    SignedPreflight {
        signature: k.sign(&value.canonical_signed_bytes())
            .as_ref().try_into().unwrap(),
        claim: value,
    }
}
fn exchange(
    policy: RootTrustedPolicy, wire: Vec<u8>,
) -> Result<InertPreflightResult, PreflightError> {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("real-af-unix.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let child = thread::spawn(move || {
        let mut peer = UnixStream::connect(&sock).unwrap();
        peer.write_all(&wire).unwrap();
        peer.shutdown(Shutdown::Write).unwrap();
    });
    let (mut accepted, _) = listener.accept().unwrap();
    let result = admit_preflight(&mut accepted, &policy, NOW);
    child.join().unwrap();
    result
}

#[test]
fn actual_linux_peer_and_ed25519_scope_produce_only_inert_preflight() {
    let proof = exchange(anchor(), signed(claim()).wire_bytes()).unwrap();
    assert_eq!(proof.observed_peer_uid, rustix::process::geteuid().as_raw());
    assert_eq!(proof.install_id, [0x11; 16]);
    assert_eq!(proof.matched_witness_revision, 7);
    assert!(proof.requires_independent_commit);
}
#[test]
fn byte_tampering_is_not_a_signed_operator_approval() {
    let mut bytes = signed(claim()).wire_bytes();
    bytes[b"Akula B1b3a helper signed preflight v1\0".len() + 2 + 16 + 2] ^= 1;
    assert_eq!(exchange(anchor(), bytes).unwrap_err(), PreflightError::BadSignature);
}
#[test]
fn properly_signed_different_host_or_plan_cannot_change_root_policy() {
    let mut c = claim();
    c.ssh_host_sha256[0] ^= 1;
    assert_eq!(exchange(anchor(), signed(c).wire_bytes()).unwrap_err(),
        PreflightError::WrongScope);
    let mut c = claim();
    c.plan_digest[0] ^= 1;
    assert_eq!(exchange(anchor(), signed(c).wire_bytes()).unwrap_err(),
        PreflightError::WrongScope);
}
#[test]
fn wrong_pinned_operator_key_refused_by_signature() {
    let mut p = anchor();
    p.pinned_approver_ed25519[0] ^= 1;
    assert_eq!(exchange(p, signed(claim()).wire_bytes()).unwrap_err(),
        PreflightError::BadSignature);
}
#[test]
fn helper_euid_and_kernel_peer_uid_are_not_user_controlled_claim_fields() {
    let mut p = anchor();
    p.service_euid = p.service_euid.wrapping_add(1);
    assert_eq!(exchange(p, signed(claim()).wire_bytes()).unwrap_err(),
        PreflightError::WrongServiceIdentity);
    let mut p = anchor();
    p.allowed_peer_uid = p.allowed_peer_uid.wrapping_add(1);
    assert_eq!(exchange(p, signed(claim()).wire_bytes()).unwrap_err(),
        PreflightError::WrongPeerIdentity);
}
#[test]
fn fresh_claim_must_be_time_bounded() {
    let mut c = claim();
    c.expires_at_unix = NOW;
    assert_eq!(exchange(anchor(), signed(c).wire_bytes()).unwrap_err(),
        PreflightError::Expired);
    let mut c = claim();
    c.issued_at_unix = NOW + 1;
    assert_eq!(exchange(anchor(), signed(c).wire_bytes()).unwrap_err(),
        PreflightError::NotYetValid);
    let mut c = claim();
    c.issued_at_unix = NOW - 1;
    c.expires_at_unix = NOW + MAX_VALIDITY_SECONDS + 50;
    assert_eq!(exchange(anchor(), signed(c).wire_bytes()).unwrap_err(),
        PreflightError::InvalidClaim);
}
#[test]
fn stale_signed_witness_revision_is_not_accepted() {
    let mut c = claim();
    c.witness_revision -= 1;
    assert_eq!(exchange(anchor(), signed(c).wire_bytes()).unwrap_err(),
        PreflightError::WrongScope);
}
#[test]
fn unknown_opcode_is_rejected_before_acceptance() {
    let mut bytes = signed(claim()).wire_bytes();
    let offset = b"Akula B1b3a helper signed preflight v1\0".len();
    bytes[offset + 1] = 2;
    assert_eq!(exchange(anchor(), bytes).unwrap_err(), PreflightError::UnsupportedOperation);
}
#[test]
fn unsupported_schema_version_is_rejected() {
    let mut bytes = signed(claim()).wire_bytes();
    let offset = b"Akula B1b3a helper signed preflight v1\0".len();
    bytes[offset] = 3;
    assert_eq!(exchange(anchor(), bytes).unwrap_err(), PreflightError::BadFrame);
}
#[test]
fn truncated_frame_is_refused() {
    let mut bytes = signed(claim()).wire_bytes();
    bytes.pop();
    assert_eq!(exchange(anchor(), bytes).unwrap_err(), PreflightError::BadFrame);
}
#[test]
fn pipelined_trailing_bytes_are_refused() {
    let mut bytes = signed(claim()).wire_bytes();
    bytes.extend_from_slice(b"RUN /bin/sh");
    assert_eq!(exchange(anchor(), bytes).unwrap_err(), PreflightError::BadFrame);
}
#[test]
fn all_zero_nonce_is_refused_even_when_signed() {
    let mut c = claim();
    c.approval_nonce = [0; 16];
    assert_eq!(exchange(anchor(), signed(c).wire_bytes()).unwrap_err(),
        PreflightError::InvalidClaim);
}
