//! B1c0: first REAL root-owned filesystem mutation from a signed, pinned FD
//! after fresh owner-signed local CAS, but ONLY in an enrolled disposable CI
//! namespace. A fixed *node file* is written; no service is launched.
//!
//! IMPORTANT: neither same-host root-controlled CAS nor this code proves
//! owner consent against hostile root or VM snapshot rollback. This module
//! CANNOT be reached from any shipped Akula app, public Node or installer IPC.
//! An accepted signed receipt cannot be reused as a public install opcode.
use crate::server_installer_cas_fence_lab::{
    check_inert_cas_fence, FenceError,
};
use crate::server_installer_os_witness_lab::{
    OsWitnessError, OsWitnessLab,
};
use crate::server_installer_pinned_release_lab::{PinError, RootVerifiedFd};
use crate::server_owner_approval_lab::{
    ExternalApprovalScope, SignedOwnerApproval, TrustedOwnerApprover,
};
use crate::server_release_floor_lab::ProposedFloor;
use crate::server_staging_lab::StageReceipt;
use crate::server_witness_lab::WitnessCheckpoint;
use ring::{constant_time, digest};
use rustix::fs::{
    flock, open, openat, renameat_with, FlockOperation, Mode, OFlags, RenameFlags,
};
use std::fs::{File, Metadata, Permissions};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
const FIXED_ROOT: &str = "/run/akula-b1c0-one-shot-ci";
const LAB_MARKER: &[u8] = b"AKULA-B1C0-ENROLLED-CI-V1\n";
const FINAL_NAME: &str = "node";
const PENDING_NAME: &str = "node.pending";

#[derive(Debug)]
pub enum FixedNodeError {
    Io(std::io::Error),
    Unprovisioned,
    WrongDirectory,
    UnsafeMarker,
    ExistingOrInterrupted,
    Witness(OsWitnessError),
    Fence(FenceError),
    Pin(PinError),
    ReadbackMismatch,
    InjectedStop, // test-only crash boundary; NEVER a successful commit
}
impl From<std::io::Error> for FixedNodeError {
    fn from(v: std::io::Error) -> Self { Self::Io(v) }
}
fn os(v: rustix::io::Errno) -> FixedNodeError {
    FixedNodeError::Io(v.into())
}
fn dir_ok(d: &Metadata, mode: u32) -> bool {
    d.is_dir() && d.uid() == 0 && d.nlink() != 0
        && d.permissions().mode() & 0o7777 == mode
}
fn file_ok(d: &Metadata, mode: u32) -> bool {
    d.is_file() && d.uid() == 0 && d.nlink() == 1
        && d.permissions().mode() & 0o7777 == mode
}
fn strict_marker(root: &File) -> Result<(), FixedNodeError> {
    let mut file = File::from(openat(
        root, "enrollment", OFlags::RDONLY | OFlags::NOFOLLOW |
            OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ).map_err(os)?);
    let before = file.metadata()?;
    if !file_ok(&before, 0o600) || before.len() != LAB_MARKER.len() as u64 {
        return Err(FixedNodeError::UnsafeMarker);
    }
    let mut actual = [0u8; LAB_MARKER.len()];
    file.read_exact(&mut actual)?;
    let after = file.metadata()?;
    if before.dev() != after.dev() || before.ino() != after.ino()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
        || actual.as_slice() != LAB_MARKER
    {
        return Err(FixedNodeError::UnsafeMarker);
    }
    Ok(())
}
fn absent(dir: &File, name: &str) -> Result<(), FixedNodeError> {
    match openat(
        dir, name, OFlags::RDONLY | OFlags::NOFOLLOW |
            OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        _ => Err(FixedNodeError::ExistingOrInterrupted),
    }
}
fn open_target() -> Result<File, FixedNodeError> {
    if rustix::process::geteuid().as_raw() != 0
        || std::env::var("GITHUB_ACTIONS").ok().as_deref() != Some("true")
        || std::env::var("RUNNER_OS").ok().as_deref() != Some("Linux")
    {
        return Err(FixedNodeError::Unprovisioned);
    }
    let root = File::from(open(
        Path::new(FIXED_ROOT),
        OFlags::RDONLY | OFlags::DIRECTORY |
            OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ).map_err(os)?);
    if !dir_ok(&root.metadata()?, 0o711) {
        return Err(FixedNodeError::WrongDirectory);
    }
    strict_marker(&root)?;
    let target = File::from(openat(
        &root, "target", OFlags::RDONLY | OFlags::DIRECTORY |
            OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ).map_err(os)?);
    if !dir_ok(&target.metadata()?, 0o755) {
        return Err(FixedNodeError::WrongDirectory);
    }
    Ok(target)
}
fn verify_readback(file: &mut File, receipt: &crate::server_release_lab::VerifiedPackageReceipt)
    -> Result<(), FixedNodeError>
{
    let mut hash = digest::Context::new(&digest::SHA256);
    let mut nread = 0u64;
    let mut buf = [0u8; 32 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 { break; }
        nread = nread.checked_add(n as u64)
            .ok_or(FixedNodeError::ReadbackMismatch)?;
        if nread > receipt.byte_length {
            return Err(FixedNodeError::ReadbackMismatch);
        }
        hash.update(&buf[..n]);
    }
    if nread != receipt.byte_length || constant_time::verify_slices_are_equal(
        hash.finish().as_ref(), &receipt.package_sha256
    ).is_err() {
        return Err(FixedNodeError::ReadbackMismatch);
    }
    Ok(())
}
/// A strictly forensic receipt for the CI-only *file* commit.
/// It is not permission to execute or activate a systemd service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixedNodeReceipt {
    pub witness_revision: u64,
    pub installed_byte_length: u64,
    pub installed_sha256: [u8; 32],
    pub no_activation_was_performed: bool,
}
/// This is the FIRST fixed-path data mutation in the isolated bootstrap chain.
/// The operation has NO externally chosen destination, command or PID.
/// A failure after the CAS commit leaves a consumed approval and a repair HOLD.
/// It does not retry and may not overwrite a candidate or pending file.
#[allow(clippy::too_many_arguments)]
pub fn commit_first_signed_node_in_ci(
    pinned: &RootVerifiedFd,
    actor: &OsWitnessLab,
    signed_owner: &SignedOwnerApproval,
    trusted_owner: &TrustedOwnerApprover,
    scope: ExternalApprovalScope,
    staged: &StageReceipt,
    proposed: ProposedFloor,
    expected_before: WitnessCheckpoint,
    helper_challenge: [u8; 32],
    now_unix: u64,
) -> Result<FixedNodeReceipt, FixedNodeError> {
    let dest = open_target()?;
    flock(&dest, FlockOperation::LockExclusive).map_err(os)?;
    commit_signed_node_locked(
        &dest, pinned, actor, signed_owner, trusted_owner, scope,
        staged, proposed, expected_before, helper_challenge, now_unix,
        CoreStop::Never,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CoreStop {
    Never,
    AfterCas,
    AfterPendingSync,
    AfterRename,
}

/// Shared transaction. Caller must hold a root-owned destination-directory
/// lock across this operation and any additional persisted receipt write.
/// This does not issue activation authority or choose any arbitrary path.
#[allow(clippy::too_many_arguments)]
pub(crate) fn commit_signed_node_locked(
    dest: &File,
    pinned: &RootVerifiedFd,
    actor: &OsWitnessLab,
    signed_owner: &SignedOwnerApproval,
    trusted_owner: &TrustedOwnerApprover,
    scope: ExternalApprovalScope,
    staged: &StageReceipt,
    proposed: ProposedFloor,
    expected_before: WitnessCheckpoint,
    helper_challenge: [u8; 32],
    now_unix: u64,
    stop: CoreStop,
) -> Result<FixedNodeReceipt, FixedNodeError> {
    absent(&dest, FINAL_NAME)?;
    absent(&dest, PENDING_NAME)?;
    pinned.valid_at(now_unix).map_err(FixedNodeError::Pin)?;
    if staged.verified_package != pinned.receipt {
        return Err(FixedNodeError::ReadbackMismatch);
    }
    if actor.current_checkpoint().map_err(FixedNodeError::Witness)? != expected_before {
        return Err(FixedNodeError::ExistingOrInterrupted);
    }
    // Sign ONLY the actual new fsynced B1b1c CAS revision. No caller-supplied
    // claim is accepted, and a retry cannot obtain another signed receipt.
    let witness = actor.attest_new_record(
        pinned, signed_owner, trusted_owner, scope, staged,
        proposed, expected_before, helper_challenge, now_unix,
    ).map_err(FixedNodeError::Witness)?;
    let after = actor.current_checkpoint().map_err(FixedNodeError::Witness)?;
    if after != witness.claim.after {
        return Err(FixedNodeError::ExistingOrInterrupted);
    }
    check_inert_cas_fence(
        pinned, signed_owner, trusted_owner, scope, staged, proposed,
        &witness, &actor.trusted_witness(), expected_before, after,
        helper_challenge, now_unix,
    ).map_err(FixedNodeError::Fence)?;
    if stop == CoreStop::AfterCas {
        return Err(FixedNodeError::InjectedStop);
    }

    // CREATE|EXCL makes any existing crash partial file a hard refusal.
    let mut tmp = File::from(openat(
        &dest, PENDING_NAME, OFlags::WRONLY | OFlags::CREATE |
            OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    ).map_err(os)?);
    if !file_ok(&tmp.metadata()?, 0o600) {
        return Err(FixedNodeError::ReadbackMismatch);
    }
    pinned.copy_rechecked_bytes(&mut tmp, now_unix)
        .map_err(FixedNodeError::Pin)?;
    tmp.set_permissions(Permissions::from_mode(0o555))?;
    tmp.sync_all()?;
    if stop == CoreStop::AfterPendingSync {
        return Err(FixedNodeError::InjectedStop);
    }
    drop(tmp);
    let mut candidate = File::from(openat(
        &dest, PENDING_NAME,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ).map_err(os)?);
    if !file_ok(&candidate.metadata()?, 0o555) {
        return Err(FixedNodeError::ReadbackMismatch);
    }
    verify_readback(&mut candidate, &pinned.receipt)?;
    // The package FD must still refer to the identical signed inode before
    // the target name is atomically published.
    pinned.valid_at(now_unix).map_err(FixedNodeError::Pin)?;
    absent(&dest, FINAL_NAME)?;
    renameat_with(
        &dest, PENDING_NAME, &dest, FINAL_NAME,
        RenameFlags::NOREPLACE,
    ).map_err(os)?;
    dest.sync_all()?;
    if stop == CoreStop::AfterRename {
        return Err(FixedNodeError::InjectedStop);
    }
    // Read back the durable namespace entry after the rename.
    let mut final_file = File::from(openat(
        &dest, FINAL_NAME,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ).map_err(os)?);
    if !file_ok(&final_file.metadata()?, 0o555) {
        return Err(FixedNodeError::ReadbackMismatch);
    }
    verify_readback(&mut final_file, &pinned.receipt)?;
    if actor.current_checkpoint().map_err(FixedNodeError::Witness)? != after {
        return Err(FixedNodeError::ExistingOrInterrupted);
    }
    Ok(FixedNodeReceipt {
        witness_revision: after.revision,
        installed_byte_length: pinned.receipt.byte_length,
        installed_sha256: pinned.receipt.package_sha256,
        no_activation_was_performed: true,
    })
}
