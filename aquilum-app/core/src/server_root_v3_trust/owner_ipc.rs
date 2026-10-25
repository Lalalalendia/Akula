//! Non-shipping one-shot AF_UNIX preview for canonical owner-signed B1b1b.
//! No listener, spawn, sudo, root command, B1d0 CAS, /opt or systemd here.
//! A root-provisioned 0600 peer UID pin is mandatory; no request can set it.
use super::*;
use crate::server_installer_ipc_lab::{verified_peer_uid, PreflightError};
use crate::server_owner_approval_lab::SignedOwnerApproval;
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PEER_PIN_FILE: &str = "b1d0.peer-uid";
const PEER_DOMAIN: &[u8] = b"AKULA-B1-E2E-07-ROOT-ALLOWED-IPC-UID-V1\0";
const PREVIEW_DOMAIN: &[u8] = b"AKULA-B1-E2E-07-OWNER-FRAME-PREVIEW-V1\0";
const MAX_FRAME_BYTES: usize = 768;
const READ_DEADLINE: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub(crate) enum OwnerIpcError {
    Trust(RootEnrollmentError),
    Peer(PreflightError),
    WrongPeer,
    InvalidPeerPin,
    InvalidFrame,
    Io(std::io::Error),
}
impl From<RootEnrollmentError> for OwnerIpcError {
    fn from(error: RootEnrollmentError) -> Self {
        Self::Trust(error)
    }
}
impl From<std::io::Error> for OwnerIpcError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
/// The explicit administrator must separately enroll the fixed UID, not
/// accept a UID from the SSH user field or the AF_UNIX request itself.
fn read_allowed_peer(trust: &File) -> Result<u32, OwnerIpcError> {
    let bytes = read_root_file(trust, PEER_PIN_FILE, 128)?;
    if bytes.len() != PEER_DOMAIN.len() + 4 || !bytes.starts_with(PEER_DOMAIN) {
        return Err(OwnerIpcError::InvalidPeerPin);
    }
    let uid = u32::from_be_bytes(
        bytes[PEER_DOMAIN.len()..]
            .try_into()
            .map_err(|_| OwnerIpcError::InvalidPeerPin)?,
    );
    if uid == 0 || uid == u32::MAX {
        return Err(OwnerIpcError::InvalidPeerPin);
    }
    Ok(uid)
}

/// This is an authenticated *preview only*. Its return type has no root
/// install capability and is not accepted as a commit receipt.
#[derive(Debug)]
pub(crate) struct InertOwnerIpcPreview {
    pub peer_uid: u32,
    pub owner: SignedOwnerApproval,
    pub requires_independent_b0_b1a_cas: bool,
}

pub(super) fn preview_from_trust_fd(
    stream: &mut UnixStream,
    trust: &File,
    now_unix: u64,
) -> Result<InertOwnerIpcPreview, OwnerIpcError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(RootEnrollmentError::NotRoot.into());
    }
    let policy = RootEnrolledV3::from_trust_fd(trust)?;
    let allowed = read_allowed_peer(trust)?;
    // Peer authority is from the Linux kernel, not the frame's "user" text.
    let peer = verified_peer_uid(stream).map_err(OwnerIpcError::Peer)?;
    if peer == 0 || peer != allowed {
        return Err(OwnerIpcError::WrongPeer);
    }
    stream.set_read_timeout(Some(READ_DEADLINE))?;
    let mut header = [0u8; PREVIEW_DOMAIN.len() + 2];
    stream.read_exact(&mut header)?;
    if !header.starts_with(PREVIEW_DOMAIN) {
        return Err(OwnerIpcError::InvalidFrame);
    }
    let len = u16::from_be_bytes(
        header[PREVIEW_DOMAIN.len()..]
            .try_into()
            .map_err(|_| OwnerIpcError::InvalidFrame)?,
    ) as usize;
    if !(65..=MAX_FRAME_BYTES).contains(&len) {
        return Err(OwnerIpcError::InvalidFrame);
    }
    let mut raw = vec![0u8; len];
    stream.read_exact(&mut raw)?;
    // Exactly one bounded frame; pipelining, trailing data and requests
    // that never close their write half are all refused.
    let mut trailing = [0u8; 1];
    if stream.read(&mut trailing)? != 0 {
        return Err(OwnerIpcError::InvalidFrame);
    }
    let owner = super::owner_frame::verify_from_trust_fd(trust, &raw, now_unix)?;
    if owner.claim.scope_binding.pinned_host_key_sha256 != policy.ssh_host_sha256 {
        return Err(OwnerIpcError::InvalidFrame);
    }
    Ok(InertOwnerIpcPreview {
        peer_uid: peer,
        owner,
        requires_independent_b0_b1a_cas: true,
    })
}

/// Production code has no listener to call this. When an independently
/// authenticated root-helper binary is installed, this fixed-root adapter
/// may be called only after protected socket creation by that helper.
/// No user-controlled root filesystem path, clock or peer pin is accepted.
#[allow(dead_code)]
pub(crate) fn preview_provisioned_once(
    stream: &mut UnixStream,
) -> Result<InertOwnerIpcPreview, OwnerIpcError> {
    let trust = open_fixed_trust_fd()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RootEnrollmentError::InvalidOwnerClock)?
        .as_secs();
    preview_from_trust_fd(stream, &trust, now)
}

#[cfg(test)]
mod tests;
