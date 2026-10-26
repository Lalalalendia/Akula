//! B1b3a Linux AF_UNIX peer-credential boundary and Ed25519 installation
//! PREFLIGHT. The only opcode is Preview. There is deliberately NO filesystem
//! mutation, executable launch, sudo, systemd or arbitrary command/path.
//! NOT a production privileged installer or proof of remote Windows consent.

use ring::signature;
use std::ffi::c_void;
use std::io::Read;
use std::mem;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

const DOMAIN: &[u8] = b"Akula B1b3a helper signed preflight v1\0";
const SCHEMA: u8 = 1;
const OP_PREVIEW: u8 = 1;
const FIELDS_BYTES: usize = 2 + 16 + 32 + 32 + 32 + 32 + 8 + 8 + 16 + 8 + 8;
const SIGNED_BYTES: usize = DOMAIN.len() + FIELDS_BYTES;
pub const WIRE_BYTES: usize = SIGNED_BYTES + 64;
pub const MAX_VALIDITY_SECONDS: u64 = 15 * 60;

#[repr(C)]
#[derive(Clone, Copy)]
struct RawLinuxUcred { pid: i32, uid: u32, gid: u32 }

unsafe extern "C" {
    fn getsockopt(
        fd: i32, level: i32, name: i32, value: *mut c_void, size: *mut u32,
    ) -> i32;
}

/// The helper's separately provisioned OS-owned policy. Its contents MUST NOT
/// originate from socket payloads, arbitrary user files or the candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RootTrustedPolicy {
    pub service_euid: u32,
    pub allowed_peer_uid: u32,
    pub pinned_approver_ed25519: [u8; 32],
    pub install_id: [u8; 16],
    pub approved_plan_digest: [u8; 32],
    pub pinned_ssh_host_sha256: [u8; 32],
    pub signed_manifest_fingerprint: [u8; 32],
    pub staged_package_sha256: [u8; 32],
    pub expected_release_sequence: u64,
    pub expected_witness_revision: u64,
}

/// No strings, paths, URLs, shell syntax or operation argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreflightClaim {
    pub install_id: [u8; 16],
    pub plan_digest: [u8; 32],
    pub ssh_host_sha256: [u8; 32],
    pub signed_manifest_fingerprint: [u8; 32],
    pub package_sha256: [u8; 32],
    pub release_sequence: u64,
    pub witness_revision: u64,
    pub approval_nonce: [u8; 16],
    pub issued_at_unix: u64,
    pub expires_at_unix: u64,
}
impl PreflightClaim {
    pub fn canonical_signed_bytes(&self) -> Vec<u8> {
        let mut wire = Vec::with_capacity(SIGNED_BYTES);
        wire.extend_from_slice(DOMAIN);
        wire.extend_from_slice(&[SCHEMA, OP_PREVIEW]);
        wire.extend_from_slice(&self.install_id);
        wire.extend_from_slice(&self.plan_digest);
        wire.extend_from_slice(&self.ssh_host_sha256);
        wire.extend_from_slice(&self.signed_manifest_fingerprint);
        wire.extend_from_slice(&self.package_sha256);
        wire.extend_from_slice(&self.release_sequence.to_be_bytes());
        wire.extend_from_slice(&self.witness_revision.to_be_bytes());
        wire.extend_from_slice(&self.approval_nonce);
        wire.extend_from_slice(&self.issued_at_unix.to_be_bytes());
        wire.extend_from_slice(&self.expires_at_unix.to_be_bytes());
        debug_assert_eq!(wire.len(), SIGNED_BYTES);
        wire
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedPreflight {
    pub claim: PreflightClaim,
    pub signature: [u8; 64],
}
impl SignedPreflight {
    pub fn wire_bytes(&self) -> Vec<u8> {
        let mut encoded = self.claim.canonical_signed_bytes();
        encoded.extend_from_slice(&self.signature);
        encoded
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreflightError {
    Socket,
    BadPeerCredentials,
    WrongServiceIdentity,
    WrongPeerIdentity,
    BadFrame,
    UnsupportedOperation,
    InvalidClaim,
    WrongScope,
    Expired,
    NotYetValid,
    BadSignature,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InertPreflightResult {
    pub observed_peer_uid: u32,
    pub install_id: [u8; 16],
    pub matched_witness_revision: u64,
    /// Never permission to install; external owner-approved CAS and package
    /// inode custody are not created or verified by this IPC laboratory.
    pub requires_independent_commit: bool,
}

fn kernel_peer(stream: &UnixStream) -> Result<RawLinuxUcred, PreflightError> {
    // Linux UAPI constants: SOL_SOCKET=1, SO_PEERCRED=17. Use a plain C
    // struct; pid=0 must not become a NonZero PID (see rustix issue #1698).
    let mut creds = RawLinuxUcred { pid: 0, uid: 0, gid: 0 };
    let mut len = mem::size_of::<RawLinuxUcred>() as u32;
    // SAFETY: stream owns a live fd, buffer is properly sized repr(C), and
    // len points to a valid writable u32.
    let ret = unsafe {
        getsockopt(stream.as_raw_fd(), 1, 17,
            (&mut creds as *mut RawLinuxUcred).cast::<c_void>(), &mut len)
    };
    if ret != 0 || len != mem::size_of::<RawLinuxUcred>() as u32 || creds.pid <= 0 {
        return Err(PreflightError::BadPeerCredentials);
    }
    Ok(creds)
}
/// Kernel-reported peer UID, not supplied by client frames or filesystem
/// metadata. Reused by opt-in read-only witness IPC.
pub fn verified_peer_uid(stream: &UnixStream) -> Result<u32, PreflightError> {
    kernel_peer(stream).map(|peer| peer.uid)
}

struct Decoder<'a> { buf: &'a [u8], cursor: usize }
impl Decoder<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], PreflightError> {
        let end = self.cursor.checked_add(N).ok_or(PreflightError::BadFrame)?;
        let bytes = self.buf.get(self.cursor..end).ok_or(PreflightError::BadFrame)?;
        self.cursor = end;
        bytes.try_into().map_err(|_| PreflightError::BadFrame)
    }
    fn u64(&mut self) -> Result<u64, PreflightError> {
        Ok(u64::from_be_bytes(self.take::<8>()?))
    }
}
fn decode_wire(wire: &[u8]) -> Result<SignedPreflight, PreflightError> {
    if wire.len() != WIRE_BYTES || !wire.starts_with(DOMAIN) {
        return Err(PreflightError::BadFrame);
    }
    let mut d = Decoder { buf: wire, cursor: DOMAIN.len() };
    if d.take::<1>()?[0] != SCHEMA { return Err(PreflightError::BadFrame); }
    if d.take::<1>()?[0] != OP_PREVIEW { return Err(PreflightError::UnsupportedOperation); }
    let claim = PreflightClaim {
        install_id: d.take::<16>()?,
        plan_digest: d.take::<32>()?,
        ssh_host_sha256: d.take::<32>()?,
        signed_manifest_fingerprint: d.take::<32>()?,
        package_sha256: d.take::<32>()?,
        release_sequence: d.u64()?,
        witness_revision: d.u64()?,
        approval_nonce: d.take::<16>()?,
        issued_at_unix: d.u64()?,
        expires_at_unix: d.u64()?,
    };
    let signature = d.take::<64>()?;
    if d.cursor != WIRE_BYTES || claim.canonical_signed_bytes() != wire[..SIGNED_BYTES] {
        return Err(PreflightError::BadFrame);
    }
    Ok(SignedPreflight { claim, signature })
}

fn verify_signed(
    request: &SignedPreflight, policy: &RootTrustedPolicy, now_unix: u64
) -> Result<(), PreflightError> {
    let c = request.claim;
    if policy.pinned_approver_ed25519 == [0; 32]
        || policy.install_id == [0; 16]
        || policy.approved_plan_digest == [0; 32]
        || policy.pinned_ssh_host_sha256 == [0; 32]
        || policy.signed_manifest_fingerprint == [0; 32]
        || policy.staged_package_sha256 == [0; 32]
        || policy.expected_release_sequence == 0
        || c.approval_nonce == [0; 16]
    { return Err(PreflightError::InvalidClaim); }

    let span = c.expires_at_unix.checked_sub(c.issued_at_unix)
        .ok_or(PreflightError::InvalidClaim)?;
    if span == 0 || span > MAX_VALIDITY_SECONDS { return Err(PreflightError::InvalidClaim); }
    if now_unix < c.issued_at_unix { return Err(PreflightError::NotYetValid); }
    if now_unix >= c.expires_at_unix { return Err(PreflightError::Expired); }

    signature::UnparsedPublicKey::new(
        &signature::ED25519, &policy.pinned_approver_ed25519
    ).verify(&c.canonical_signed_bytes(), &request.signature)
        .map_err(|_| PreflightError::BadSignature)?;

    if c.install_id != policy.install_id
        || c.plan_digest != policy.approved_plan_digest
        || c.ssh_host_sha256 != policy.pinned_ssh_host_sha256
        || c.signed_manifest_fingerprint != policy.signed_manifest_fingerprint
        || c.package_sha256 != policy.staged_package_sha256
        || c.release_sequence != policy.expected_release_sequence
        || c.witness_revision != policy.expected_witness_revision
    { return Err(PreflightError::WrongScope); }
    Ok(())
}

/// Requires an actual AF_UNIX socket: the kernel's peer UID is checked
/// before parsing. Only Preview opcode is recognized; no side effects.
pub fn admit_preflight(
    stream: &mut UnixStream, policy: &RootTrustedPolicy, now_unix: u64,
) -> Result<InertPreflightResult, PreflightError> {
    if rustix::process::geteuid().as_raw() != policy.service_euid {
        return Err(PreflightError::WrongServiceIdentity);
    }
    let peer = kernel_peer(stream)?;
    if peer.uid != policy.allowed_peer_uid {
        return Err(PreflightError::WrongPeerIdentity);
    }
    stream.set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| PreflightError::Socket)?;
    let mut wire = [0u8; WIRE_BYTES];
    stream.read_exact(&mut wire).map_err(|_| PreflightError::BadFrame)?;
    // Require EOF after one fixed-size packet, no extra bytes or pipelining.
    let mut extra = [0u8; 1];
    if stream.read(&mut extra).map_err(|_| PreflightError::BadFrame)? != 0 {
        return Err(PreflightError::BadFrame);
    }
    let signed = decode_wire(&wire)?;
    verify_signed(&signed, policy, now_unix)?;
    Ok(InertPreflightResult {
        observed_peer_uid: peer.uid,
        install_id: signed.claim.install_id,
        matched_witness_revision: signed.claim.witness_revision,
        requires_independent_commit: true,
    })
}
