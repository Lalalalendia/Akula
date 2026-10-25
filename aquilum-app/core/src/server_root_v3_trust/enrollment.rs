//! Operator-only first-time B0 v3 trust enrollment, NEVER an SSH endpoint.
//! Run only as an independently authorized UID0 administrator using an
//! independently authenticated binary. Reads the exact root-owned candidate
//! from /root, never from argv, stdin, an app setting or a remote request.
//!
//! Two-file publication is not atomic. Any interrupted state remains HOLD
//! and requires a separate human root repair; no automatic retry or overwrite.
use super::*;
use crate::server_installer_os_witness_lab::{OsWitnessError, OsWitnessLab};
use rustix::fs::{openat, renameat_with, RenameFlags};
use std::io::Write;

const ADMIN_CANDIDATE: &str = "akula-b0v3-policy.candidate";
const POLICY_PENDING: &str = "b0v3.policy.pending";
const POLICY_FINAL: &str = "b0v3.policy";
const MARKER_PENDING: &str = "b0v3.enrollment.pending";
const MARKER_FINAL: &str = "b0v3.enrollment";

#[derive(Debug)]
pub enum AdminEnrollError {
    Read(RootEnrollmentError),
    Witness(OsWitnessError),
    IncompleteOrAlreadyEnrolled,
    RootIo(std::io::Error),
}

impl From<RootEnrollmentError> for AdminEnrollError {
    fn from(error: RootEnrollmentError) -> Self {
        Self::Read(error)
    }
}
impl From<std::io::Error> for AdminEnrollError {
    fn from(error: std::io::Error) -> Self {
        Self::RootIo(error)
    }
}
fn syscall(error: rustix::io::Errno) -> AdminEnrollError {
    AdminEnrollError::RootIo(error.into())
}
fn no_object(dir: &File, name: &str) -> Result<(), AdminEnrollError> {
    match openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Ok(_) => Err(AdminEnrollError::IncompleteOrAlreadyEnrolled),
        Err(_) => Err(AdminEnrollError::IncompleteOrAlreadyEnrolled),
    }
}
fn publish_one(
    dir: &File,
    pending: &str,
    final_name: &str,
    data: &[u8],
) -> Result<(), AdminEnrollError> {
    let mut file = File::from(
        openat(
            dir,
            pending,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(syscall)?,
    );
    if !root_file_ok(&file.metadata()?, 0o600, MAX_POLICY_SIZE)
        || data.len() as u64 > MAX_POLICY_SIZE
    {
        return Err(AdminEnrollError::IncompleteOrAlreadyEnrolled);
    }
    file.write_all(data)?;
    file.sync_all()?;
    drop(file);
    renameat_with(dir, pending, dir, final_name, RenameFlags::NOREPLACE).map_err(syscall)?;
    dir.sync_all()?;
    Ok(())
}

/// This private transactional primitive is tested in a disposable root-owned
/// directory. Production may call it ONLY after verifying the fixed /root
/// candidate, OS-owned witness and independently authorized administrator.
pub(super) fn enroll_once_from_admin_bytes(trust: &File, candidate: &[u8]) -> Result<(), AdminEnrollError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(AdminEnrollError::Read(RootEnrollmentError::NotRoot));
    }
    if !root_dir_ok(&trust.metadata()?, 0o700) {
        return Err(AdminEnrollError::Read(RootEnrollmentError::UntrustedPath));
    }
    // Never create trust over an unprovisioned or CI-only witness identity.
    if read_root_file(trust, "enrollment", 128)? != WITNESS_ENROLLMENT {
        return Err(AdminEnrollError::Read(
            RootEnrollmentError::InvalidEnrollment,
        ));
    }
    let _policy = RootEnrolledV3::parse(candidate)?;
    for leaf in [POLICY_PENDING, POLICY_FINAL, MARKER_PENDING, MARKER_FINAL] {
        no_object(trust, leaf)?;
    }
    // The policy is fully durable before the enrollment marker is published.
    // Crashes after policy rename leave marker absent => loader refuses.
    publish_one(trust, POLICY_PENDING, POLICY_FINAL, candidate)?;
    publish_one(trust, MARKER_PENDING, MARKER_FINAL, ENROLLMENT)?;
    RootEnrolledV3::from_trust_fd(trust)?;
    Ok(())
}

/// Root-only, fixed-path administrative ceremony, not a client install RPC.
///
/// Preconditions: an external trusted administrator already installed a
/// verified helper binary, securely enrolled the B1d0 witness + ledger, and
/// placed the independently reviewed exact binary policy at
/// /root/akula-b0v3-policy.candidate with root ownership and mode 0600.
/// No part of this function obtains or grants elevation.
pub fn admin_enroll_fixed_root_once() -> Result<(), AdminEnrollError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(AdminEnrollError::Read(RootEnrollmentError::NotRoot));
    }
    let slash = File::from(
        open(
            Path::new("/"),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(syscall)?,
    );
    if !root_dir_ok(&slash.metadata()?, 0o755) {
        return Err(AdminEnrollError::Read(RootEnrollmentError::UntrustedPath));
    }
    let home = File::from(
        openat(
            &slash,
            "root",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(syscall)?,
    );
    if !root_dir_ok(&home.metadata()?, 0o700) {
        return Err(AdminEnrollError::Read(RootEnrollmentError::UntrustedPath));
    }
    let candidate = read_root_file(&home, ADMIN_CANDIDATE, MAX_POLICY_SIZE)?;
    RootEnrolledV3::parse(&candidate)?;
    // Validate existing protected witness authority before publishing B0
    // policy. A temporary test seed or an absent journal is not enrollment.
    let _witness =
        OsWitnessLab::open_provisioned_first_install().map_err(AdminEnrollError::Witness)?;
    let trust = open_fixed_trust_fd()?;
    enroll_once_from_admin_bytes(&trust, &candidate)
}

#[cfg(test)]
mod tests;
