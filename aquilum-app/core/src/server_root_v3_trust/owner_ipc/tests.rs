use super::*;
use crate::server_owner_approval_wire::{OwnerApprovalWireV1, OwnerFloorWire};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::fs::{self, Permissions};
use std::io::Write;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::thread;
use tempfile::{tempdir, TempDir};

const NOW: u64 = 1_800_000_000;
const TEST_PEER_UID: u32 = 65534;
const OWNER_ID: &str = "owner-ipc-e2e";

fn protected(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, Permissions::from_mode(0o600)).unwrap();
}
fn fixture() -> (TempDir, File, Ed25519KeyPair) {
    let temp = tempdir().unwrap();
    // Disposable tests alone let the unprivileged child traverse to the
    // test socket; the real root trust directory remains private 0700.
    fs::set_permissions(temp.path(), Permissions::from_mode(0o711)).unwrap();
    let trust_path = temp.path().join("trust");
    fs::create_dir(&trust_path).unwrap();
    fs::set_permissions(&trust_path, Permissions::from_mode(0o700)).unwrap();
    let owner = Ed25519KeyPair::from_seed_unchecked(&[0x92; 32]).unwrap();
    let mut policy = DOMAIN.to_vec();
    policy.push(3);
    policy.push(OWNER_ID.len() as u8);
    policy.extend_from_slice(OWNER_ID.as_bytes());
    policy.extend_from_slice(owner.public_key().as_ref());
    policy.extend_from_slice(&[0x22; 32]); // separately pinned host
    policy.extend_from_slice(&[0x23; 32]); // separately pinned helper
    let publisher = b"publisher-ipc";
    policy.push(publisher.len() as u8);
    policy.extend_from_slice(publisher);
    policy.extend_from_slice(&[0x44; 32]);
    protected(&trust_path.join("enrollment"), WITNESS_ENROLLMENT);
    protected(&trust_path.join("b0v3.enrollment"), ENROLLMENT);
    protected(&trust_path.join("b0v3.policy"), &policy);
    let mut peer_pin = PEER_DOMAIN.to_vec();
    peer_pin.extend_from_slice(&TEST_PEER_UID.to_be_bytes());
    protected(&trust_path.join(PEER_PIN_FILE), &peer_pin);
    (temp, File::open(trust_path).unwrap(), owner)
}
fn signed_frame(owner: &Ed25519KeyPair) -> Vec<u8> {
    let claim = OwnerApprovalWireV1 {
        approver_id: OWNER_ID.into(),
        install_id: [0x11; 16],
        plan_digest: [0x21; 32],
        ssh_host_sha256: [0x22; 32],
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
        issued_at_unix: NOW - 10,
        expires_at_unix: NOW + 60,
    };
    let mut bytes = claim.canonical_bytes();
    let signature = owner.sign(&bytes);
    bytes.extend_from_slice(signature.as_ref());
    bytes
}
fn request(frame: &[u8]) -> Vec<u8> {
    let mut bytes = PREVIEW_DOMAIN.to_vec();
    bytes.extend_from_slice(&(frame.len() as u16).to_be_bytes());
    bytes.extend_from_slice(frame);
    bytes
}
fn spawn_unprivileged_socket_sender(socket: &Path, bytes: &[u8]) -> Child {
    // Same disposable Python used by the existing Ubuntu root-stage CI.
    // It does not parse or trust the message; it merely delivers bytes
    // on a real unprivileged AF_UNIX connection.
    let code = "import socket,sys\ns=socket.socket(socket.AF_UNIX)\ns.connect(sys.argv[1])\nb=sys.stdin.buffer.read()\ns.sendall(b)\ns.shutdown(socket.SHUT_WR)\ns.close()\n";
    let mut child = Command::new("/usr/bin/python3")
        .arg("-I")
        .arg("-c")
        .arg(code)
        .arg(socket)
        .uid(TEST_PEER_UID)
        .gid(TEST_PEER_UID)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    child
}
fn accept_bounded(listener: &UnixListener) -> UnixStream {
    listener.set_nonblocking(true).unwrap();
    for _ in 0..150 {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(err) => panic!("test socket accept failed: {err}"),
        }
    }
    panic!("test unprivileged peer did not connect in 3 seconds")
}
fn socket_fixture(temp: &TempDir) -> (UnixListener, std::path::PathBuf) {
    let path = temp.path().join("preview.sock");
    let listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, Permissions::from_mode(0o666)).unwrap();
    (listener, path)
}
fn receive_once(
    listener: &UnixListener,
    socket: &Path,
    trust: &File,
    bytes: &[u8],
) -> Result<InertOwnerIpcPreview, OwnerIpcError> {
    let mut child = spawn_unprivileged_socket_sender(socket, bytes);
    let mut stream = accept_bounded(listener);
    let result = preview_from_trust_fd(&mut stream, trust, NOW);
    assert!(child.wait().unwrap().success(), "disposable sender failed");
    result
}

#[test]
fn unprivileged_real_kernel_peer_can_preview_signed_owner_but_never_install() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    let (temp, trust, owner) = fixture();
    let (listener, socket) = socket_fixture(&temp);
    let bytes = request(&signed_frame(&owner));
    let preview = receive_once(&listener, &socket, &trust, &bytes).unwrap();
    assert_eq!(preview.peer_uid, TEST_PEER_UID);
    assert_eq!(
        preview.owner.claim.scope_binding.pinned_host_key_sha256,
        [0x22; 32]
    );
    assert!(preview.requires_independent_b0_b1a_cas);
    // No CAS or install is performed, so a repeated PREVIEW remains inert.
    let replay = receive_once(&listener, &socket, &trust, &bytes).unwrap();
    assert_eq!(
        replay.owner.claim.approval_nonce,
        preview.owner.claim.approval_nonce
    );
    assert_eq!(fs::read_dir(temp.path().join("trust")).unwrap().count(), 4);
}

#[test]
fn corrupt_truncated_and_extended_frames_never_pass_kernel_peer_admission() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    let (temp, trust, owner) = fixture();
    let (listener, socket) = socket_fixture(&temp);
    let good = request(&signed_frame(&owner));
    let mut bad_signature = good.clone();
    *bad_signature.last_mut().unwrap() ^= 1;
    let mut excess = good.clone();
    excess.push(0);
    let mut wrong_opcode = good.clone();
    wrong_opcode[0] ^= 1;
    let mut huge = good.clone();
    huge[PREVIEW_DOMAIN.len()] = 0xff;
    huge[PREVIEW_DOMAIN.len() + 1] = 0xff;
    // A correctly re-signed host substitution must still fail the
    // separately enrolled root-owned SSH host pin.
    let parsed = crate::server_owner_approval_wire::decode::SignedOwnerWireFrame::decode(
        &signed_frame(&owner),
    )
    .unwrap();
    let mut changed_host = parsed.claim;
    changed_host.ssh_host_sha256[0] ^= 1;
    let mut wrong_host = changed_host.canonical_bytes();
    let signature = owner.sign(&wrong_host);
    wrong_host.extend_from_slice(signature.as_ref());
    for bad in [
        bad_signature,
        good[..good.len() - 1].to_vec(),
        excess,
        wrong_opcode,
        huge,
        request(&wrong_host),
    ] {
        assert!(receive_once(&listener, &socket, &trust, &bad).is_err());
    }
}

#[test]
fn root_peer_and_missing_or_unsafe_uid_pin_are_never_trusted() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    let (temp, trust, _) = fixture();
    let (mut root_peer, _other) = UnixStream::pair().unwrap();
    assert!(matches!(
        preview_from_trust_fd(&mut root_peer, &trust, NOW),
        Err(OwnerIpcError::WrongPeer)
    ));
    let path = temp.path().join("trust").join(PEER_PIN_FILE);
    fs::set_permissions(&path, Permissions::from_mode(0o644)).unwrap();
    let (mut peer, _other) = UnixStream::pair().unwrap();
    assert!(preview_from_trust_fd(&mut peer, &trust, NOW).is_err());
    fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
    fs::rename(&path, temp.path().join("trust/saved-peer")).unwrap();
    symlink("saved-peer", &path).unwrap();
    let (mut peer, _other) = UnixStream::pair().unwrap();
    assert!(preview_from_trust_fd(&mut peer, &trust, NOW).is_err());
    fs::remove_file(&path).unwrap();
    let mut invalid_pin = PEER_DOMAIN.to_vec();
    invalid_pin.extend_from_slice(&0u32.to_be_bytes());
    protected(&path, &invalid_pin);
    let (mut peer, _other) = UnixStream::pair().unwrap();
    assert!(matches!(
        preview_from_trust_fd(&mut peer, &trust, NOW),
        Err(OwnerIpcError::InvalidPeerPin)
    ));
}

#[test]
fn ordinary_unprivileged_caller_cannot_run_root_peer_admission() {
    if rustix::process::geteuid().as_raw() == 0 {
        return;
    }
    let (_temp, trust, _) = fixture();
    let (mut client, _other) = UnixStream::pair().unwrap();
    assert!(matches!(
        preview_from_trust_fd(&mut client, &trust, NOW),
        Err(OwnerIpcError::Trust(RootEnrollmentError::NotRoot))
    ));
}
