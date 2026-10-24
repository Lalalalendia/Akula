//! B1d0: DIRECT first-Node file commit into enrolled root-owned /opt, reusing
//! B1c0's verified FD + owner-signed B1b1c CAS transaction. NO auto-activation.
//!
//! A successful commit leaves a durable local receipt and a *HOLD*; only a
//! separately authenticated owner/activation authority may start systemd.
//! Same-host root is trusted. Full disk rollback/hostile root is not defended.
//! This module has NO socket, remote command, arbitrary path or UI entrypoint.
use crate::server_installer_fixed_node_lab::{
    commit_signed_node_locked, CoreStop, FixedNodeError,
};
use crate::server_installer_os_witness_lab::{OsWitnessError, OsWitnessLab};
use crate::server_installer_pinned_release_lab::RootVerifiedFd;
use crate::server_installer_cas_fence_lab::signed_owner_sha256;
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
use std::fs::{File, Metadata};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

const PROD_LEAF: &str = "akula-node";
const CI_LEAF: &str = "akula-b1d0-ci-node";
const PROD_ENROLLMENT: &[u8] = b"AKULA-B1D0-PROVISIONED-FIRST-INSTALL-V1\n";
const CI_ENROLLMENT: &[u8] = b"AKULA-B1D0-ISOLATED-CI-FIRST-INSTALL-V1\n";
const RECORD_DOMAIN: &[u8] = b"AKULA-B1D0-DURABLE-LOCAL-HOLD-V1\0";
const RECORD_HASH_DOMAIN: &str = "Akula B1d0 durable local receipt checksum v1";
const NODE_NAME: &str = "node";
const NODE_PENDING: &str = "node.pending";
const RECEIPT_NAME: &str = "node.receipt";
const RECEIPT_PENDING: &str = "node.receipt.pending";
const MAX_PACKAGE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Destination { ProvisionedOpt, DisposableCi }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashPoint {
    None,
    AfterCas,
    AfterPendingSync,
    AfterNodeRename,
    AfterReceiptPendingSync,
}

#[derive(Debug)]
pub enum DurableError {
    Io(std::io::Error),
    NotRoot,
    UntrustedRoot,
    Unenrolled,
    NeedsRepair,
    InvalidReceipt,
    Witness(OsWitnessError),
    FileCommit(FixedNodeError),
    InjectedStop,
}
impl From<std::io::Error> for DurableError {
    fn from(v: std::io::Error) -> Self { Self::Io(v) }
}
fn os(v: rustix::io::Errno) -> DurableError {
    DurableError::Io(v.into())
}
fn file_ok(m: &Metadata, mode: u32) -> bool {
    m.is_file() && m.uid() == 0 && m.nlink() == 1
        && m.permissions().mode() & 0o7777 == mode
}
fn dir_ok(m: &Metadata, mode: u32) -> bool {
    m.is_dir() && m.uid() == 0 && m.nlink() != 0
        && m.permissions().mode() & 0o7777 == mode
}
fn checked_open(dir: &File, name: &str) -> Result<Option<File>, DurableError> {
    match openat(
        dir, name, OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK
            | OFlags::CLOEXEC, Mode::empty(),
    ) {
        Ok(fd) => Ok(Some(File::from(fd))),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(_) => Err(DurableError::NeedsRepair),
    }
}
fn absent(dir: &File, name: &str) -> Result<(), DurableError> {
    if checked_open(dir, name)?.is_some() {
        Err(DurableError::NeedsRepair)
    } else { Ok(()) }
}
fn open_root(dest: Destination) -> Result<File, DurableError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(DurableError::NotRoot);
    }
    if dest == Destination::DisposableCi && (
        std::env::var("GITHUB_ACTIONS").ok().as_deref() != Some("true")
        || std::env::var("RUNNER_OS").ok().as_deref() != Some("Linux")
        || std::env::var("AKULA_B1D0_CI").ok().as_deref()
            != Some("STRICT_ROOT_ONE_SHOT_FILE_ONLY")
    ) {
        return Err(DurableError::Unenrolled);
    }
    // Absolute /opt is a root-owned 0755 directory, not a symlink.
    let opt = File::from(open(
        Path::new("/opt"),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW
            | OFlags::CLOEXEC, Mode::empty(),
    ).map_err(os)?);
    if !dir_ok(&opt.metadata()?, 0o755) {
        return Err(DurableError::UntrustedRoot);
    }
    let (leaf, expected_marker) = match dest {
        Destination::ProvisionedOpt => (PROD_LEAF, PROD_ENROLLMENT),
        Destination::DisposableCi => (CI_LEAF, CI_ENROLLMENT),
    };
    let root = File::from(openat(
        &opt, leaf, OFlags::RDONLY | OFlags::DIRECTORY
            | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty(),
    ).map_err(os)?);
    if !dir_ok(&root.metadata()?, 0o755) {
        return Err(DurableError::UntrustedRoot);
    }
    let mut marker = checked_open(&root, "enrollment")?
        .ok_or(DurableError::Unenrolled)?;
    let before = marker.metadata()?;
    if !file_ok(&before, 0o600) || before.len() != expected_marker.len() as u64 {
        return Err(DurableError::Unenrolled);
    }
    let mut data = vec![0u8; expected_marker.len()];
    marker.read_exact(&mut data)?;
    let after = marker.metadata()?;
    if data.as_slice() != expected_marker
        || before.dev() != after.dev() || before.ino() != after.ino()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        return Err(DurableError::Unenrolled);
    }
    Ok(root)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LocalReceipt {
    revision: u64,
    size: u64,
    package_sha256: [u8; 32],
    manifest_fingerprint: [u8; 32],
    record_tip: [u8; 32],
    floor_fingerprint: [u8; 32],
    plan_digest: [u8; 32],
    host_digest: [u8; 32],
    install_id: [u8; 16],
    approval_nonce: [u8; 16],
    signed_owner_sha256: [u8; 32],
}
impl LocalReceipt {
    fn encode(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(RECORD_DOMAIN.len() + 16 + 7*32 + 2*16 + 32);
        out.extend_from_slice(RECORD_DOMAIN);
        out.extend_from_slice(&self.revision.to_be_bytes());
        out.extend_from_slice(&self.size.to_be_bytes());
        for h in [
            self.package_sha256, self.manifest_fingerprint, self.record_tip,
            self.floor_fingerprint, self.plan_digest, self.host_digest,
            self.signed_owner_sha256,
        ] {
            out.extend_from_slice(&h);
        }
        out.extend_from_slice(&self.install_id);
        out.extend_from_slice(&self.approval_nonce);
        let mut hash = blake3::Hasher::new_derive_key(RECORD_HASH_DOMAIN);
        hash.update(&out);
        out.extend_from_slice(hash.finalize().as_bytes());
        out
    }
    fn decode(raw: &[u8]) -> Result<Self, DurableError> {
        const BODY: usize = 16 + 7*32 + 2*16;
        if raw.len() != RECORD_DOMAIN.len() + BODY + 32
            || !raw.starts_with(RECORD_DOMAIN)
        {
            return Err(DurableError::InvalidReceipt);
        }
        let data_len = raw.len() - 32;
        let mut hash = blake3::Hasher::new_derive_key(RECORD_HASH_DOMAIN);
        hash.update(&raw[..data_len]);
        if constant_time::verify_slices_are_equal(
            hash.finalize().as_bytes(), &raw[data_len..],
        ).is_err() {
            return Err(DurableError::InvalidReceipt);
        }
        let mut p = RECORD_DOMAIN.len();
        fn take<const N: usize>(raw: &[u8], cursor: &mut usize)
            -> Result<[u8; N], DurableError>
        {
            let end = cursor.checked_add(N).ok_or(DurableError::InvalidReceipt)?;
            let bytes: [u8; N] = raw.get(*cursor..end)
                .ok_or(DurableError::InvalidReceipt)?
                .try_into().map_err(|_| DurableError::InvalidReceipt)?;
            *cursor = end;
            Ok(bytes)
        }
        let revision = u64::from_be_bytes(take::<8>(raw, &mut p)?);
        let size = u64::from_be_bytes(take::<8>(raw, &mut p)?);
        let record = Self {
            revision, size,
            package_sha256: take::<32>(raw, &mut p)?,
            manifest_fingerprint: take::<32>(raw, &mut p)?,
            record_tip: take::<32>(raw, &mut p)?,
            floor_fingerprint: take::<32>(raw, &mut p)?,
            plan_digest: take::<32>(raw, &mut p)?,
            host_digest: take::<32>(raw, &mut p)?,
            signed_owner_sha256: take::<32>(raw, &mut p)?,
            install_id: take::<16>(raw, &mut p)?,
            approval_nonce: take::<16>(raw, &mut p)?,
        };
        if p != data_len || revision == 0 || size == 0
            || size > MAX_PACKAGE_BYTES || record.record_tip == [0; 32]
            || record.package_sha256 == [0; 32]
            || record.manifest_fingerprint == [0; 32]
        {
            return Err(DurableError::InvalidReceipt);
        }
        Ok(record)
    }
}
fn read_receipt(dir: &File) -> Result<Option<LocalReceipt>, DurableError> {
    let Some(mut file) = checked_open(dir, RECEIPT_NAME)? else { return Ok(None) };
    let meta = file.metadata()?;
    let expected_len = LocalReceipt {
        revision: 1, size: 1, package_sha256: [1;32],
        manifest_fingerprint: [1;32], record_tip: [1;32],
        floor_fingerprint: [1;32], plan_digest: [1;32],
        host_digest: [1;32], install_id: [1;16],
        approval_nonce: [1;16], signed_owner_sha256: [1;32],
    }.encode().len();
    if !file_ok(&meta, 0o600) || meta.len() != expected_len as u64 {
        return Err(DurableError::InvalidReceipt);
    }
    let mut raw = Vec::new();
    (&mut file).take(expected_len as u64 + 1).read_to_end(&mut raw)?;
    if file.metadata()?.len() != expected_len as u64 {
        return Err(DurableError::InvalidReceipt);
    }
    Ok(Some(LocalReceipt::decode(&raw)?))
}
fn persist_receipt(dir: &File, receipt: LocalReceipt,
    stop: CrashPoint) -> Result<(), DurableError>
{
    absent(dir, RECEIPT_NAME)?;
    absent(dir, RECEIPT_PENDING)?;
    let mut temp = File::from(openat(
        dir, RECEIPT_PENDING, OFlags::WRONLY | OFlags::CREATE
            | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    ).map_err(os)?);
    if !file_ok(&temp.metadata()?, 0o600) {
        return Err(DurableError::NeedsRepair);
    }
    temp.write_all(&receipt.encode())?;
    temp.sync_all()?;
    if stop == CrashPoint::AfterReceiptPendingSync {
        return Err(DurableError::InjectedStop);
    }
    drop(temp);
    renameat_with(
        dir, RECEIPT_PENDING, dir, RECEIPT_NAME, RenameFlags::NOREPLACE,
    ).map_err(os)?;
    dir.sync_all()?;
    if read_receipt(dir)? != Some(receipt) {
        return Err(DurableError::InvalidReceipt);
    }
    Ok(())
}
fn read_signed_node(dir: &File, expected: &LocalReceipt)
    -> Result<(), DurableError>
{
    let mut file = checked_open(dir, NODE_NAME)?
        .ok_or(DurableError::NeedsRepair)?;
    let meta = file.metadata()?;
    if !file_ok(&meta, 0o555) || meta.len() != expected.size {
        return Err(DurableError::NeedsRepair);
    }
    let mut digest = digest::Context::new(&digest::SHA256);
    let mut nread = 0u64;
    let mut buf = [0u8; 32 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 { break; }
        nread = nread.checked_add(n as u64).ok_or(DurableError::NeedsRepair)?;
        if nread > expected.size { return Err(DurableError::NeedsRepair); }
        digest.update(&buf[..n]);
    }
    if nread != expected.size || constant_time::verify_slices_are_equal(
        digest.finish().as_ref(), &expected.package_sha256,
    ).is_err() {
        return Err(DurableError::NeedsRepair);
    }
    let after = file.metadata()?;
    if meta.dev() != after.dev() || meta.ino() != after.ino()
        || meta.ctime() != after.ctime()
        || meta.ctime_nsec() != after.ctime_nsec()
    {
        return Err(DurableError::NeedsRepair);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DurableStatus {
    Empty,
    /// Local signed-file hash and CAS checkpoint match. This is explicitly
    /// NOT authorization to activate or restore a service after reboot.
    LocallyConsistentHold { revision: u64, sha256: [u8;32] },
}
fn inspect_locked(dir: &File, actor: &OsWitnessLab)
    -> Result<DurableStatus, DurableError>
{
    absent(dir, NODE_PENDING)?;
    absent(dir, RECEIPT_PENDING)?;
    let file = checked_open(dir, NODE_NAME)?;
    let receipt = read_receipt(dir)?;
    match (file, receipt) {
        (None, None) => {
            if actor.current_checkpoint().map_err(DurableError::Witness)?
                == WitnessCheckpoint::genesis()
            { Ok(DurableStatus::Empty) } else { Err(DurableError::NeedsRepair) }
        }
        (Some(_), Some(saved)) => {
            let checkpoint = actor.current_checkpoint().map_err(DurableError::Witness)?;
            if checkpoint.revision != saved.revision
                || checkpoint.record_tip != saved.record_tip
                || checkpoint.accepted_floor.floor.signed_manifest_fingerprint
                    != saved.manifest_fingerprint
                || checkpoint.accepted_floor.floor.signed_manifest_fingerprint
                    != saved.floor_fingerprint
            {
                return Err(DurableError::NeedsRepair);
            }
            read_signed_node(dir, &saved)?;
            Ok(DurableStatus::LocallyConsistentHold {
                revision: saved.revision, sha256: saved.package_sha256,
            })
        }
        _ => Err(DurableError::NeedsRepair),
    }
}

/// Opens the root-enrolled production /opt/akula-node directory. This stays
/// crate-private until the independent owner/release trust authority is
/// loaded from fixed protected OS state and installed via a secure helper.
/// NO UI or IPC route currently calls it.
#[allow(dead_code, clippy::too_many_arguments)]
pub(crate) fn commit_first_provisioned_opt(
    pinned: &RootVerifiedFd,
    owner: &SignedOwnerApproval,
    trusted_owner: &TrustedOwnerApprover,
    scope: ExternalApprovalScope,
    staged: &StageReceipt,
    proposed: ProposedFloor,
    expected_before: WitnessCheckpoint,
    challenge: [u8;32],
    now_unix: u64,
) -> Result<DurableStatus, DurableError> {
    // Actor origin is OS-enrolled and fixed under /var/lib/akula. A caller
    // cannot pass its own test witness actor to a production /opt commit.
    let actor = OsWitnessLab::open_provisioned_first_install()
        .map_err(DurableError::Witness)?;
    commit_in(Destination::ProvisionedOpt, pinned, &actor, owner, trusted_owner,
        scope, staged, proposed, expected_before, challenge, now_unix, CrashPoint::None)
}
/// On restart: receipt and executable bytes can be inspected only alongside
/// the SAME independently root-provisioned production CAS ledger. Never
/// grants automatic service activation after reboot.
#[allow(dead_code)]
pub fn inspect_provisioned_opt_hold() -> Result<DurableStatus, DurableError> {
    let actor = OsWitnessLab::open_provisioned_first_install()
        .map_err(DurableError::Witness)?;
    let root = open_root(Destination::ProvisionedOpt)?;
    flock(&root, FlockOperation::LockExclusive).map_err(os)?;
    inspect_locked(&root, &actor)
}
#[allow(clippy::too_many_arguments)]
fn commit_in(
    which: Destination,
    pinned: &RootVerifiedFd,
    actor: &OsWitnessLab,
    owner: &SignedOwnerApproval,
    trusted_owner: &TrustedOwnerApprover,
    scope: ExternalApprovalScope,
    staged: &StageReceipt,
    proposed: ProposedFloor,
    expected_before: WitnessCheckpoint,
    challenge: [u8;32],
    now_unix: u64,
    stop: CrashPoint,
) -> Result<DurableStatus, DurableError> {
    let root = open_root(which)?;
    flock(&root, FlockOperation::LockExclusive).map_err(os)?;
    absent(&root, RECEIPT_NAME)?;
    absent(&root, RECEIPT_PENDING)?;
    // Do not claim a clean first install with any partial/corrupt leftover.
    absent(&root, NODE_NAME)?;
    absent(&root, NODE_PENDING)?;
    let core_stop = match stop {
        CrashPoint::None | CrashPoint::AfterReceiptPendingSync => CoreStop::Never,
        CrashPoint::AfterCas => CoreStop::AfterCas,
        CrashPoint::AfterPendingSync => CoreStop::AfterPendingSync,
        CrashPoint::AfterNodeRename => CoreStop::AfterRename,
    };
    let committed = commit_signed_node_locked(
        &root, pinned, actor, owner, trusted_owner, scope, staged, proposed,
        expected_before, challenge, now_unix, core_stop,
    ).map_err(|e| match e {
        FixedNodeError::InjectedStop => DurableError::InjectedStop,
        other => DurableError::FileCommit(other),
    })?;
    let checkpoint = actor.current_checkpoint().map_err(DurableError::Witness)?;
    if checkpoint.revision != committed.witness_revision
        || checkpoint.record_tip == [0;32]
    {
        return Err(DurableError::NeedsRepair);
    }
    let receipt = LocalReceipt {
        revision: checkpoint.revision,
        size: committed.installed_byte_length,
        package_sha256: committed.installed_sha256,
        manifest_fingerprint: pinned.receipt.signed_manifest_fingerprint,
        record_tip: checkpoint.record_tip,
        floor_fingerprint: checkpoint.accepted_floor.floor.signed_manifest_fingerprint,
        plan_digest: scope.approved_plan_digest.0,
        host_digest: scope.pinned_host_key_sha256,
        install_id: scope.install_id.0,
        approval_nonce: owner.claim.approval_nonce,
        signed_owner_sha256: signed_owner_sha256(owner),
    };
    persist_receipt(&root, receipt, stop)?;
    match inspect_locked(&root, actor)? {
        hold @ DurableStatus::LocallyConsistentHold { .. } => Ok(hold),
        DurableStatus::Empty => Err(DurableError::NeedsRepair),
    }
}

/// Only a strict, isolated CI proof can choose this otherwise-inaccessible
/// test root. Does not touch /opt/akula-node or activate anything.
#[allow(clippy::too_many_arguments)]
pub fn commit_first_ci_opt(
    pinned: &RootVerifiedFd,
    actor: &OsWitnessLab,
    owner: &SignedOwnerApproval,
    trusted_owner: &TrustedOwnerApprover,
    scope: ExternalApprovalScope,
    staged: &StageReceipt,
    proposed: ProposedFloor,
    expected_before: WitnessCheckpoint,
    challenge: [u8;32],
    now_unix: u64,
    stop: CrashPoint,
) -> Result<DurableStatus, DurableError> {
    commit_in(Destination::DisposableCi, pinned, actor, owner, trusted_owner,
        scope, staged, proposed, expected_before, challenge, now_unix, stop)
}
pub fn inspect_ci_hold(actor: &OsWitnessLab) -> Result<DurableStatus, DurableError> {
    let root = open_root(Destination::DisposableCi)?;
    flock(&root, FlockOperation::LockExclusive).map_err(os)?;
    inspect_locked(&root, actor)
}
