//! Fixed /opt/akula-node B1d0 install destination enrollment.
//! Administrator-only, one-shot, non-shipping. This NEVER commits a Node,
//! starts systemd, accepts SSH arguments or grants activation permission.
use super::*;
use crate::server_installer_os_witness_lab::{OsWitnessError, OsWitnessLab};
use rustix::fs::{flock, mkdirat, renameat_with, FlockOperation, RenameFlags};
use std::io::Write;

const OPT_LEAF: &str = "akula-node";
const OPT_MARKER: &str = "enrollment";
const OPT_MARKER_PENDING: &str = "enrollment.pending";
const OPT_ENROLLMENT: &[u8] = b"AKULA-B1D0-PROVISIONED-FIRST-INSTALL-V1\n";

#[derive(Debug)]
pub enum OptFirstEnrollError {
    NotRoot,
    ParentUntrusted,
    UnenrolledWitness(OsWitnessError),
    UnenrolledPolicy(RootEnrollmentError),
    ExistingOrInterrupted,
    MarkerMismatch,
    Io(std::io::Error),
}
fn syscall(error: rustix::io::Errno) -> OptFirstEnrollError {
    OptFirstEnrollError::Io(error.into())
}

/// Testable only by passing a preauthenticated disposable /opt-shaped
/// directory FD. Production alone opens the real fixed absolute /opt.
fn first_opt_destination(opt: &File) -> Result<(), OptFirstEnrollError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(OptFirstEnrollError::NotRoot);
    }
    if !root_dir_ok(&opt.metadata().map_err(OptFirstEnrollError::Io)?, 0o755) {
        return Err(OptFirstEnrollError::ParentUntrusted);
    }
    flock(opt, FlockOperation::LockExclusive).map_err(syscall)?;
    let result = (|| {
        mkdirat(
            opt,
            OPT_LEAF,
            Mode::RUSR
                | Mode::WUSR
                | Mode::XUSR
                | Mode::RGRP
                | Mode::XGRP
                | Mode::ROTH
                | Mode::XOTH,
        )
        .map_err(|_| OptFirstEnrollError::ExistingOrInterrupted)?;
        opt.sync_all().map_err(OptFirstEnrollError::Io)?;
        let destination = File::from(
            openat(
                opt,
                OPT_LEAF,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(syscall)?,
        );
        if !root_dir_ok(
            &destination.metadata().map_err(OptFirstEnrollError::Io)?,
            0o755,
        ) {
            return Err(OptFirstEnrollError::ParentUntrusted);
        }
        // The destination begins EMPTY. Marker publication is the only
        // state change after creation, and it happens LAST.
        let mut marker = File::from(
            openat(
                &destination,
                OPT_MARKER_PENDING,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(|_| OptFirstEnrollError::ExistingOrInterrupted)?,
        );
        if !root_file_ok(
            &marker.metadata().map_err(OptFirstEnrollError::Io)?,
            0o600,
            128,
        ) {
            return Err(OptFirstEnrollError::ParentUntrusted);
        }
        marker
            .write_all(OPT_ENROLLMENT)
            .map_err(OptFirstEnrollError::Io)?;
        marker.sync_all().map_err(OptFirstEnrollError::Io)?;
        drop(marker);
        renameat_with(
            &destination,
            OPT_MARKER_PENDING,
            &destination,
            OPT_MARKER,
            RenameFlags::NOREPLACE,
        )
        .map_err(|_| OptFirstEnrollError::ExistingOrInterrupted)?;
        destination.sync_all().map_err(OptFirstEnrollError::Io)?;
        if read_root_file(&destination, OPT_MARKER, 128)
            .map_err(OptFirstEnrollError::UnenrolledPolicy)?
            != OPT_ENROLLMENT
        {
            return Err(OptFirstEnrollError::MarkerMismatch);
        }
        Ok(())
    })();
    let unlock = flock(opt, FlockOperation::Unlock).map_err(syscall);
    unlock?;
    result
}

/// Root-admin fixed command: requires B1d0 witness and B0 v3 publisher/owner
/// enrollment BEFORE publishing an empty /opt target. No arbitrary path or
/// remote key is an authorization input, and there is no reset/retry.
pub fn admin_enroll_fixed_opt_once() -> Result<(), OptFirstEnrollError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(OptFirstEnrollError::NotRoot);
    }
    OsWitnessLab::open_provisioned_first_install()
        .map_err(OptFirstEnrollError::UnenrolledWitness)?;
    load_provisioned_root_v3_policy().map_err(OptFirstEnrollError::UnenrolledPolicy)?;
    observed_root_host().map_err(OptFirstEnrollError::UnenrolledPolicy)?;
    let opt = File::from(
        open(
            Path::new("/opt"),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(syscall)?,
    );
    first_opt_destination(&opt)
}

#[cfg(test)]
mod tests;
