//! One-shot root-only first B1a release approval. No SSH, UI or install authority.
//! A separately authorized administrator must provision an independently signed
//! stage and a fixed reviewed candidate. Interrupted writes remain HOLD.
use super::*;
use crate::server_installer_os_witness_lab::{OsWitnessError, OsWitnessLab};
use rustix::fs::{flock, open, openat, renameat_with, FlockOperation, Mode, OFlags, RenameFlags};
use std::io::Write;

const ADMIN_CANDIDATE: &str = "akula-b1a-first.approval.candidate";
const APPROVAL_PENDING: &str = "b1a.first.approval.pending";
const MARKER_PENDING: &str = "b1a.first.enrollment.pending";

#[derive(Debug)]
pub enum FirstReleaseEnrollError {
    Release(FirstReleaseError),
    Witness(OsWitnessError),
    ExistingOrInterrupted,
    Io(std::io::Error),
}
impl From<FirstReleaseError> for FirstReleaseEnrollError {
    fn from(value: FirstReleaseError) -> Self {
        Self::Release(value)
    }
}
impl From<std::io::Error> for FirstReleaseEnrollError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
fn syscall(err: rustix::io::Errno) -> FirstReleaseEnrollError {
    FirstReleaseEnrollError::Io(err.into())
}
fn refuse_existing(dir: &File, leaf: &str) -> Result<(), FirstReleaseEnrollError> {
    match openat(
        dir,
        leaf,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        _ => Err(FirstReleaseEnrollError::ExistingOrInterrupted),
    }
}
fn publish_one(
    dir: &File,
    pending: &str,
    final_name: &str,
    bytes: &[u8],
) -> Result<(), FirstReleaseEnrollError> {
    let mut file = File::from(
        openat(
            dir,
            pending,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(syscall)?,
    );
    if bytes.is_empty() || bytes.len() > 256 || !root_file_ok(&file.metadata()?, 0o600, 256) {
        return Err(FirstReleaseEnrollError::ExistingOrInterrupted);
    }
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    renameat_with(dir, pending, dir, final_name, RenameFlags::NOREPLACE).map_err(syscall)?;
    dir.sync_all()?;
    Ok(())
}

/// Primitive exercised only on disposable root-owned directories in CI.
/// It does not derive trust from the caller's candidate or from staged bytes:
/// independent B0 v3 policy determines the release publisher Ed25519 key.
pub(super) fn enroll_once_from_reviewed_bytes(
    trust: &File,
    candidate: &[u8],
    now_unix: u64,
) -> Result<(), FirstReleaseEnrollError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(FirstReleaseError::Trust(RootEnrollmentError::NotRoot).into());
    }
    if !root_dir_ok(&trust.metadata()?, 0o700) {
        return Err(FirstReleaseError::Trust(RootEnrollmentError::UntrustedPath).into());
    }
    flock(trust, FlockOperation::LockExclusive).map_err(syscall)?;
    let result = (|| {
        let policy = RootEnrolledV3::from_trust_fd(trust).map_err(FirstReleaseError::Trust)?;
        let approval = FirstReleaseApproval::decode(candidate)?;
        for leaf in [
            APPROVAL_PENDING,
            FIRST_APPROVAL_FILE,
            MARKER_PENDING,
            FIRST_APPROVAL_MARKER,
        ] {
            refuse_existing(trust, leaf)?;
        }
        let stage = File::from(
            openat(
                trust,
                FIRST_RELEASE_STAGE,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|e| FirstReleaseError::Trust(RootEnrollmentError::Io(e.into())))?,
        );
        if !root_dir_ok(&stage.metadata()?, 0o700) {
            return Err(FirstReleaseError::UnsafeStage.into());
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
        let verified =
            verify_root_staged_dir(&stage, &anchor, now_unix).map_err(FirstReleaseError::Verify)?;
        if verified.receipt.byte_length != approval.package_size {
            return Err(FirstReleaseError::InvalidApproval.into());
        }
        verified
            .valid_at(now_unix)
            .map_err(FirstReleaseError::Verify)?;
        // Strict publication order: approval fully durable BEFORE marker.
        // A crash or preexisting pending name cannot silently retry.
        publish_one(trust, APPROVAL_PENDING, FIRST_APPROVAL_FILE, candidate)?;
        publish_one(
            trust,
            MARKER_PENDING,
            FIRST_APPROVAL_MARKER,
            FIRST_APPROVAL_MARKER_BYTES,
        )?;
        verify_first_from_trust_fd(trust, now_unix)?;
        Ok(())
    })();
    flock(trust, FlockOperation::Unlock).map_err(syscall)?;
    result
}

/// Explicit administrator-only fixed-path enrollment; NEVER called by Akula UI.
/// The verified candidate is root-owned 0600 under /root. It cannot be supplied
/// by a host response, arbitrary remote filepath, package manifest or argv.
pub fn admin_enroll_first_b1a_fixed_root_once() -> Result<(), FirstReleaseEnrollError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(FirstReleaseError::Trust(RootEnrollmentError::NotRoot).into());
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
        return Err(FirstReleaseError::Trust(RootEnrollmentError::UntrustedPath).into());
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
        return Err(FirstReleaseError::Trust(RootEnrollmentError::UntrustedPath).into());
    }
    let candidate =
        read_root_file(&home, ADMIN_CANDIDATE, 256).map_err(FirstReleaseError::Trust)?;
    FirstReleaseApproval::decode(&candidate)?;
    OsWitnessLab::open_provisioned_first_install().map_err(FirstReleaseEnrollError::Witness)?;
    let trust = open_fixed_trust_fd().map_err(FirstReleaseError::Trust)?;
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| FirstReleaseError::NoSystemClock)?
        .as_secs();
    enroll_once_from_reviewed_bytes(&trust, &candidate, now_unix)
}

#[cfg(test)]
mod tests;
