//! Root-admin-only signed first-release staging into fixed trust storage.
//! No SSH/stdin-controlled paths, remote fetch, privilege grant or execution.
//! The protected source has to be independently provisioned under /root.
//! Partial staging is deliberately left pending and requires manual root repair.
use super::*;
use crate::server_installer_os_witness_lab::{OsWitnessError, OsWitnessLab};
use crate::server_release_lab::MAX_NODE_PACKAGE_BYTES;
use rustix::fs::{
    flock, mkdirat, open, openat, renameat_with, FlockOperation, Mode, OFlags, RenameFlags,
};
use std::io::{Read, Write};

const ADMIN_SOURCE: &str = "akula-b1a-first-source";
const ADMIN_CANDIDATE: &str = "akula-b1a-first.approval.candidate";
const STAGE_PENDING: &str = "b1a-first-stage.pending";

#[derive(Debug)]
pub enum FirstStageError {
    Release(FirstReleaseError),
    Witness(OsWitnessError),
    ExistingOrInterrupted,
    UntrustedSource,
    Io(std::io::Error),
}
impl From<FirstReleaseError> for FirstStageError {
    fn from(error: FirstReleaseError) -> Self {
        Self::Release(error)
    }
}
impl From<RootEnrollmentError> for FirstStageError {
    fn from(error: RootEnrollmentError) -> Self {
        Self::Release(FirstReleaseError::Trust(error))
    }
}
impl From<PinError> for FirstStageError {
    fn from(error: PinError) -> Self {
        Self::Release(FirstReleaseError::Verify(error))
    }
}
impl From<std::io::Error> for FirstStageError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
fn syscall(error: rustix::io::Errno) -> FirstStageError {
    FirstStageError::Io(error.into())
}
fn absent(dir: &File, leaf: &str) -> Result<(), FirstStageError> {
    match openat(
        dir,
        leaf,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        _ => Err(FirstStageError::ExistingOrInterrupted),
    }
}
fn new_protected_file(dir: &File, name: &str) -> Result<File, FirstStageError> {
    let f = File::from(
        openat(
            dir,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR,
        )
        .map_err(syscall)?,
    );
    if !root_file_ok(&f.metadata()?, 0o400, MAX_NODE_PACKAGE_BYTES) {
        return Err(FirstStageError::UntrustedSource);
    }
    Ok(f)
}
/// Clone only bounded, root-owned, single-link canonical manifest/signature
/// files. The post-copy B1a signature verifier reopens these published bytes.
fn copy_small_file(
    source: &File,
    destination: &File,
    name: &str,
    max: u64,
) -> Result<(), FirstStageError> {
    let input = File::from(
        openat(
            source,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(syscall)?,
    );
    let before = input.metadata()?;
    if !root_file_ok(&before, 0o400, max) || before.len() == 0 {
        return Err(FirstStageError::UntrustedSource);
    }
    let mut bytes = Vec::new();
    (&input).take(max + 1).read_to_end(&mut bytes)?;
    if !stable(&before, &input.metadata()?) || bytes.len() as u64 != before.len() {
        return Err(FirstStageError::UntrustedSource);
    }
    let mut output = new_protected_file(destination, name)?;
    output.write_all(&bytes)?;
    output.sync_all()?;
    Ok(())
}
/// No signed review or owner authorization comes from the source directory.
/// This root-pinned transaction is data custody only; no executable activation.
pub(super) fn stage_from_admin_fds(
    trust: &File,
    admin: &File,
    now_unix: u64,
) -> Result<(), FirstStageError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(RootEnrollmentError::NotRoot.into());
    }
    if !root_dir_ok(&trust.metadata()?, 0o700) || !root_dir_ok(&admin.metadata()?, 0o700) {
        return Err(FirstStageError::UntrustedSource);
    }
    flock(trust, FlockOperation::LockExclusive).map_err(syscall)?;
    let result = (|| {
        let policy = RootEnrolledV3::from_trust_fd(trust)?;
        let candidate = read_root_file(admin, ADMIN_CANDIDATE, 256)?;
        let approval = FirstReleaseApproval::decode(&candidate)?;
        // No staging after any first-release approval or partial publish.
        for leaf in [
            STAGE_PENDING,
            FIRST_RELEASE_STAGE,
            "b1a.first.approval.pending",
            FIRST_APPROVAL_FILE,
            "b1a.first.enrollment.pending",
            FIRST_APPROVAL_MARKER,
        ] {
            absent(trust, leaf)?;
        }
        let source = File::from(
            openat(
                admin,
                ADMIN_SOURCE,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(syscall)?,
        );
        if !root_dir_ok(&source.metadata()?, 0o700) {
            return Err(FirstStageError::UntrustedSource);
        }
        let anchor = RootPinnedReleaseTrust {
            trusted_signer: TrustedReleaseSigner {
                signer_id: policy.release_signer_id,
                ed25519_public: policy.release_pub,
            },
            previous_floor: ReleaseFloor::initial(),
            approved_manifest_fingerprint: approval.manifest_fingerprint,
            approved_package_sha256: approval.package_sha256,
            approved_release_sequence: approval.sequence,
        };
        let verified = verify_root_staged_dir(&source, &anchor, now_unix)?;
        if verified.receipt.byte_length != approval.package_size {
            return Err(FirstReleaseError::InvalidApproval.into());
        }

        // Exclusive pending state and fsync BEFORE copying; a crash leaves
        // a hard refusal rather than replacing an incomplete or stale stage.
        mkdirat(trust, STAGE_PENDING, Mode::RUSR | Mode::WUSR | Mode::XUSR).map_err(syscall)?;
        trust.sync_all()?;
        let pending = File::from(
            openat(
                trust,
                STAGE_PENDING,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(syscall)?,
        );
        if !root_dir_ok(&pending.metadata()?, 0o700) {
            return Err(FirstStageError::UntrustedSource);
        }
        copy_small_file(&source, &pending, "manifest", 256)?;
        copy_small_file(&source, &pending, "signature", 64)?;
        let mut node = new_protected_file(&pending, "node")?;
        verified.copy_rechecked_bytes(&mut node, now_unix)?;
        node.sync_all()?;
        drop(node);
        pending.sync_all()?;
        // Verify the copied bytes, not only the original source FD.
        let staged = verify_root_staged_dir(&pending, &anchor, now_unix)?;
        if staged.receipt.byte_length != approval.package_size {
            return Err(FirstReleaseError::InvalidApproval.into());
        }
        staged.valid_at(now_unix)?;
        drop(staged);
        renameat_with(
            trust,
            STAGE_PENDING,
            trust,
            FIRST_RELEASE_STAGE,
            RenameFlags::NOREPLACE,
        )
        .map_err(syscall)?;
        trust.sync_all()?;
        // The published directory must remain readable to the independent
        // first-release verifier, but STILL lacks the approval marker.
        let published = File::from(
            openat(
                trust,
                FIRST_RELEASE_STAGE,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(syscall)?,
        );
        let final_receipt = verify_root_staged_dir(&published, &anchor, now_unix)?;
        if final_receipt.receipt.byte_length != approval.package_size {
            return Err(FirstReleaseError::InvalidApproval.into());
        }
        Ok(())
    })();
    flock(trust, FlockOperation::Unlock).map_err(syscall)?;
    result
}

/// Requires a previously verified standalone binary run by an authorized UID0
/// administrator. Uses ONLY fixed source and destination paths, never an RPC.
pub fn admin_stage_first_b1a_fixed_root_once() -> Result<(), FirstStageError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(RootEnrollmentError::NotRoot.into());
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
        return Err(FirstStageError::UntrustedSource);
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
        return Err(FirstStageError::UntrustedSource);
    }
    OsWitnessLab::open_provisioned_first_install().map_err(FirstStageError::Witness)?;
    let trust = open_fixed_trust_fd()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| FirstReleaseError::NoSystemClock)?
        .as_secs();
    stage_from_admin_fds(&trust, &home, now)
}

#[cfg(test)]
mod tests;
