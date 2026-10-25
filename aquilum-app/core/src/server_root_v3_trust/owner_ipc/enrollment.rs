//! Independently authorized, one-shot root-owned B1d0 AF_UNIX peer pin.
//! No SSH request, API, sudo, useradd, root installer or service activation.
//! UID alone can be recycled; the fixed local non-login identity is also
//! verified on enrollment and on every production preview admission.
use super::*;
use crate::server_installer_os_witness_lab::{OsWitnessError, OsWitnessLab};
use rustix::fs::{
    flock, open, openat, renameat_with, FlockOperation, Mode, OFlags, RenameFlags,
};
use std::io::{Read, Write};

const ADMIN_CANDIDATE: &str = "akula-b1d0-peer-uid.candidate";
const PEER_PENDING: &str = "b1d0.peer-uid.pending";
const SERVICE_ACCOUNT: &str = "akula-provision";
const SERVICE_HOME: &str = "/nonexistent";
const MAX_PASSWD_BYTES: u64 = 128 * 1024;

#[derive(Debug)]
pub enum PeerEnrollError {
    Trust(RootEnrollmentError),
    Witness(OsWitnessError),
    AccountMismatch,
    InvalidCandidate,
    AlreadyEnrolledOrInterrupted,
    Io(std::io::Error),
}
impl From<RootEnrollmentError> for PeerEnrollError {
    fn from(error: RootEnrollmentError) -> Self {
        Self::Trust(error)
    }
}
impl From<std::io::Error> for PeerEnrollError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
fn syscall(error: rustix::io::Errno) -> PeerEnrollError {
    PeerEnrollError::Io(error.into())
}
fn decode_candidate(raw: &[u8]) -> Result<u32, PeerEnrollError> {
    if raw.len() != PEER_DOMAIN.len() + 4 || !raw.starts_with(PEER_DOMAIN) {
        return Err(PeerEnrollError::InvalidCandidate);
    }
    let uid = u32::from_be_bytes(
        raw[PEER_DOMAIN.len()..]
            .try_into()
            .map_err(|_| PeerEnrollError::InvalidCandidate)?,
    );
    if uid == 0 || uid == 65534 || uid == u32::MAX {
        return Err(PeerEnrollError::InvalidCandidate);
    }
    Ok(uid)
}
fn parse_local_account(passwd: &[u8], uid: u32) -> Result<(), PeerEnrollError> {
    if uid == 0 || uid == 65534 || uid == u32::MAX {
        return Err(PeerEnrollError::AccountMismatch);
    }
    let text = std::str::from_utf8(passwd).map_err(|_| PeerEnrollError::AccountMismatch)?;
    let mut found = false;
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() != 7 {
            return Err(PeerEnrollError::AccountMismatch);
        }
        let record_uid = fields[2]
            .parse::<u32>()
            .map_err(|_| PeerEnrollError::AccountMismatch)?;
        let gid = fields[3]
            .parse::<u32>()
            .map_err(|_| PeerEnrollError::AccountMismatch)?;
        if fields[0] == SERVICE_ACCOUNT {
            if found
                || record_uid != uid
                || gid == 0
                || fields[1] != "x"
                || fields[5] != SERVICE_HOME
                || !matches!(fields[6], "/usr/sbin/nologin" | "/sbin/nologin" | "/bin/false")
            {
                return Err(PeerEnrollError::AccountMismatch);
            }
            found = true;
        } else if record_uid == uid {
            // A second local username sharing the UID is forbidden.
            return Err(PeerEnrollError::AccountMismatch);
        }
    }
    if !found {
        return Err(PeerEnrollError::AccountMismatch);
    }
    Ok(())
}
/// Only the root-owned local /etc/passwd; NSS/LDAP, request-side usernames,
/// SSH login users and environment-selected paths cannot nominate a peer.
pub(super) fn fixed_service_account_matches(uid: u32) -> Result<(), PeerEnrollError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(RootEnrollmentError::NotRoot.into());
    }
    let root = File::from(
        open(
            Path::new("/"),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(syscall)?,
    );
    if !root_dir_ok(&root.metadata()?, 0o755) {
        return Err(PeerEnrollError::AccountMismatch);
    }
    let etc = File::from(
        openat(
            &root,
            "etc",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(syscall)?,
    );
    if !root_dir_ok(&etc.metadata()?, 0o755) {
        return Err(PeerEnrollError::AccountMismatch);
    }
    let passwd = File::from(
        openat(
            &etc,
            "passwd",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(syscall)?,
    );
    let before = passwd.metadata()?;
    if !root_file_ok(&before, 0o644, MAX_PASSWD_BYTES) || before.len() == 0 {
        return Err(PeerEnrollError::AccountMismatch);
    }
    let mut bytes = Vec::new();
    (&passwd).take(MAX_PASSWD_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != before.len() || !stable(&before, &passwd.metadata()?) {
        return Err(PeerEnrollError::AccountMismatch);
    }
    parse_local_account(&bytes, uid)
}
fn absent(dir: &File, leaf: &str) -> Result<(), PeerEnrollError> {
    match openat(
        dir,
        leaf,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        _ => Err(PeerEnrollError::AlreadyEnrolledOrInterrupted),
    }
}

/// Data-only, one-use, root-locked publication. The caller has already
/// reviewed a fixed root-owned candidate and an independent local account.
/// An interrupted pending name remains HOLD; no cleanup or overwrite.
pub(super) fn enroll_once_from_admin_bytes(
    trust: &File,
    candidate: &[u8],
) -> Result<(), PeerEnrollError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(RootEnrollmentError::NotRoot.into());
    }
    if !root_dir_ok(&trust.metadata()?, 0o700) {
        return Err(RootEnrollmentError::UntrustedPath.into());
    }
    let uid = decode_candidate(candidate)?;
    flock(trust, FlockOperation::LockExclusive).map_err(syscall)?;
    let result = (|| {
        RootEnrolledV3::from_trust_fd(trust)?;
        absent(trust, PEER_PENDING)?;
        absent(trust, PEER_PIN_FILE)?;
        let mut pending = File::from(
            openat(
                trust,
                PEER_PENDING,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(syscall)?,
        );
        if !root_file_ok(&pending.metadata()?, 0o600, 128) {
            return Err(PeerEnrollError::AlreadyEnrolledOrInterrupted);
        }
        pending.write_all(candidate)?;
        pending.sync_all()?;
        drop(pending);
        renameat_with(
            trust,
            PEER_PENDING,
            trust,
            PEER_PIN_FILE,
            RenameFlags::NOREPLACE,
        )
        .map_err(syscall)?;
        trust.sync_all()?;
        if read_allowed_peer(trust).map_err(|_| PeerEnrollError::InvalidCandidate)? != uid {
            return Err(PeerEnrollError::InvalidCandidate);
        }
        Ok(())
    })();
    flock(trust, FlockOperation::Unlock).map_err(syscall)?;
    result
}

/// One-time administrator command, not an installer RPC. The fixed root-only
/// candidate must be prepared independently and authenticated by the human
/// operator. No CLI args, stdin, environment-based paths or user creation.
pub fn admin_enroll_first_peer_uid_fixed_root_once() -> Result<(), PeerEnrollError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(RootEnrollmentError::NotRoot.into());
    }
    let root = File::from(
        open(
            Path::new("/"),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(syscall)?,
    );
    if !root_dir_ok(&root.metadata()?, 0o755) {
        return Err(PeerEnrollError::AccountMismatch);
    }
    let home = File::from(
        openat(
            &root,
            "root",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(syscall)?,
    );
    if !root_dir_ok(&home.metadata()?, 0o700) {
        return Err(PeerEnrollError::AccountMismatch);
    }
    let candidate = read_root_file(&home, ADMIN_CANDIDATE, 128)?;
    let uid = decode_candidate(&candidate)?;
    // This must be the named, dedicated, local NON-LOGIN account, not a
    // reused UID, 'nobody', a general SSH login or an NSS-supplied identity.
    fixed_service_account_matches(uid)?;
    OsWitnessLab::open_provisioned_first_install().map_err(PeerEnrollError::Witness)?;
    let trust = open_fixed_trust_fd()?;
    enroll_once_from_admin_bytes(&trust, &candidate)
}

#[cfg(test)]
mod tests;
