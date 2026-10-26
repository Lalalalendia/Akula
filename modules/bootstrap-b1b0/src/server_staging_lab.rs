//! B1b0: Linux-only, opt-in, inert signed-artifact staging laboratory.
//!
//! No shell, systemd, SSH, root elevation, package execution, network, or
//! mutation of Akula's installed desktop, Sync Core, File Gate or user vault.
//! A caller must independently authenticate the release via B1a and obtain
//! real operator approval. This lab only records a digest of that approval;
//! possession of the digest is NOT a proof of consent.
//!
//! Security boundaries:
//! - The directory must be private (0700), mounted on a filesystem where
//!   fsync(directory) and renameat are meaningful, and owned by the lab actor.
//! - Directory-FD-relative O_NOFOLLOW opens avoid symlink traversal of stage
//!   entries. The caller is responsible for trusting the parent directory.
//! - flock serializes cooperating B1b0 writers, NOT a hostile owner/root.
//! - A verified staged package is still not kernel-sealed or executable.
//! - Crash leftovers fail closed with NeedsRepair; no automatic deletion.
//! - Release floor persistence, host identity, plan CAS, trusted clock,
//!   power-loss proof and systemd activation are deliberately NOT here.

use crate::server_release_lab::{
    ReleaseError, VerifiedPackageReceipt, VerifiedRelease,
};
use rustix::fs::{self, FlockOperation, Mode, OFlags};
use std::fs::{File, Permissions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::Mutex;

const JOURNAL_DOMAIN: &[u8] = b"AKULA-B1B0-STAGED-JOURNAL-V1\0";
const JOURNAL_HASH_DOMAIN: &str = "Akula B1b0 private staging journal checksum v1";
const MAX_JOURNAL_BYTES: u64 = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstallId(pub [u8; 16]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApprovedPlanDigest(pub [u8; 32]);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageReceipt {
    pub install_id: InstallId,
    pub approved_plan_digest: ApprovedPlanDigest,
    pub verified_package: VerifiedPackageReceipt,
    pub idempotent_replay: bool,
}

#[derive(Debug)]
pub enum StageError {
    Io(io::Error),
    Release(ReleaseError),
    UnsafeDirectory,
    UnsafeArtifact,
    InvalidInstallId,
    InvalidApproval,
    SizeMismatch,
    NeedsRepair,
    JournalConflict,
    PoisonedLock,
}

impl From<io::Error> for StageError {
    fn from(value: io::Error) -> Self { Self::Io(value) }
}

impl From<ReleaseError> for StageError {
    fn from(value: ReleaseError) -> Self { Self::Release(value) }
}

fn os_error(err: rustix::io::Errno) -> StageError {
    StageError::Io(err.into())
}

fn hex_install_id(id: InstallId) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut name = String::with_capacity(32);
    for octet in id.0 {
        name.push(HEX[(octet >> 4) as usize] as char);
        name.push(HEX[(octet & 0x0f) as usize] as char);
    }
    name
}

fn entry_names(id: InstallId) -> (String, String, String, String) {
    let hex = hex_install_id(id);
    (
        format!("{hex}.pending"),
        format!("{hex}.verified"),
        format!("{hex}.journal.pending"),
        format!("{hex}.journal"),
    )
}

fn journal_bytes(
    install_id: InstallId,
    approval: ApprovedPlanDigest,
    receipt: &VerifiedPackageReceipt,
) -> Vec<u8> {
    let mut data = Vec::with_capacity(220);
    data.extend_from_slice(JOURNAL_DOMAIN);
    data.extend_from_slice(&install_id.0);
    data.extend_from_slice(&approval.0);
    data.extend_from_slice(&receipt.byte_length.to_be_bytes());
    data.extend_from_slice(&receipt.package_sha256);
    data.extend_from_slice(&receipt.signed_manifest_fingerprint);
    data.extend_from_slice(&receipt.proposed_floor.sequence.to_be_bytes());
    let v = receipt.proposed_floor.version;
    for part in [v.major, v.minor, v.patch] {
        data.extend_from_slice(&part.to_be_bytes());
    }
    data.extend_from_slice(&receipt.proposed_floor.signed_manifest_fingerprint);
    let mut digest = blake3::Hasher::new_derive_key(JOURNAL_HASH_DOMAIN);
    digest.update(&data);
    data.extend_from_slice(digest.finalize().as_bytes());
    data
}

pub struct PrivateStagingLab {
    dir: File,
    local_lock: Mutex<()>,
}

impl PrivateStagingLab {
    pub fn open(path: &Path) -> Result<Self, StageError> {
        let dir = File::from(
            fs::open(
                path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ).map_err(os_error)?
        );
        let meta = dir.metadata()?;
        let mode = meta.permissions().mode() & 0o777;
        if !meta.is_dir() || mode != 0o700 || meta.nlink() == 0
            || meta.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(StageError::UnsafeDirectory);
        }
        Ok(Self { dir, local_lock: Mutex::new(()) })
    }

    fn read_entry(&self, name: &str) -> Result<File, StageError> {
        Ok(File::from(fs::openat(
            &self.dir,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ).map_err(os_error)?))
    }

    fn create_entry(&self, name: &str) -> Result<File, StageError> {
        Ok(File::from(fs::openat(
            &self.dir,
            name,
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL
                | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        ).map_err(os_error)?))
    }

    fn exists(&self, name: &str) -> Result<bool, StageError> {
        match self.read_entry(name) {
            Ok(_) => Ok(true),
            Err(StageError::Io(e)) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn read_journal(&self, name: &str) -> Result<Vec<u8>, StageError> {
        let mut file = self.read_entry(name)?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.nlink() != 1
            || meta.uid() != rustix::process::geteuid().as_raw()
            || (meta.permissions().mode() & 0o777) != 0o600
        {
            return Err(StageError::JournalConflict);
        }
        let mut data = Vec::new();
        (&mut file).take(MAX_JOURNAL_BYTES + 1).read_to_end(&mut data)?;
        if data.len() as u64 > MAX_JOURNAL_BYTES {
            return Err(StageError::JournalConflict);
        }
        Ok(data)
    }

    fn verify_staged(
        &self,
        name: &str,
        verified: &VerifiedRelease,
    ) -> Result<VerifiedPackageReceipt, StageError> {
        let mut file = self.read_entry(name)?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.nlink() != 1
            || meta.uid() != rustix::process::geteuid().as_raw()
            || (meta.permissions().mode() & 0o777) != 0o400
        {
            return Err(StageError::UnsafeArtifact);
        }
        Ok(verified.verify_package_stream(&mut file)?)
    }

    fn stage_locked<R: Read>(
        &self,
        id: InstallId,
        approval: ApprovedPlanDigest,
        verified: &VerifiedRelease,
        source: &mut R,
    ) -> Result<StageReceipt, StageError> {
        let (pending, package, journal_pending, journal) = entry_names(id);
        let has_journal = self.exists(&journal)?;
        let has_package = self.exists(&package)?;
        // An orphan temporary file or half-committed transaction requires
        // explicit recovery. Never unlink a potentially active writer's file.
        if self.exists(&pending)? || self.exists(&journal_pending)? ||
            has_package != has_journal {
            return Err(StageError::NeedsRepair);
        }
        if has_package {
            let receipt = self.verify_staged(&package, verified)?;
            let expected = journal_bytes(id, approval, &receipt);
            if self.read_journal(&journal)? != expected {
                return Err(StageError::JournalConflict);
            }
            return Ok(StageReceipt {
                install_id: id, approved_plan_digest: approval,
                verified_package: receipt, idempotent_replay: true,
            });
        }

        let mut staged = self.create_entry(&pending)?;
        let expected_len = verified.manifest().package_size;
        // Read at most signed length + one byte, so unbounded input is refused.
        let copied = io::copy(
            &mut source.take(expected_len + 1),
            &mut staged,
        )?;
        if copied != expected_len {
            return Err(StageError::SizeMismatch);
        }
        staged.sync_all()?;
        staged.seek(SeekFrom::Start(0))?;
        let receipt = verified.verify_package_stream(&mut staged)?;
        staged.set_permissions(Permissions::from_mode(0o400))?;
        staged.sync_all()?;
        fs::renameat_with(
            &self.dir, pending.as_str(), &self.dir, package.as_str(),
            fs::RenameFlags::NOREPLACE,
        ).map_err(os_error)?;
        self.dir.sync_all()?;
        drop(staged);

        // Reopen by the pinned directory fd and rehash before persisting the
        // inert journal. Do not create an activation authority here.
        let readback = self.verify_staged(&package, verified)?;
        if receipt != readback {
            return Err(StageError::UnsafeArtifact);
        }

        let record = journal_bytes(id, approval, &receipt);
        let mut txn = self.create_entry(&journal_pending)?;
        txn.write_all(&record)?;
        txn.sync_all()?;
        fs::renameat_with(
            &self.dir, journal_pending.as_str(), &self.dir, journal.as_str(),
            fs::RenameFlags::NOREPLACE,
        ).map_err(os_error)?;
        self.dir.sync_all()?;
        Ok(StageReceipt {
            install_id: id, approved_plan_digest: approval,
            verified_package: receipt, idempotent_replay: false,
        })
    }

    /// Stage a B1a-authenticated release in a *private disposable laboratory*.
    /// The returned receipt cannot execute or activate the file. Exact
    /// idempotent repeats are admitted; partial records are refused.
    pub fn stage<R: Read>(
        &self,
        id: InstallId,
        approval: ApprovedPlanDigest,
        verified: &VerifiedRelease,
        source: &mut R,
    ) -> Result<StageReceipt, StageError> {
        if id.0.iter().all(|byte| *byte == 0) {
            return Err(StageError::InvalidInstallId);
        }
        if approval.0.iter().all(|byte| *byte == 0) {
            return Err(StageError::InvalidApproval);
        }
        let _guard = self.local_lock.lock().map_err(|_| StageError::PoisonedLock)?;
        fs::flock(&self.dir, FlockOperation::LockExclusive).map_err(os_error)?;
        let result = self.stage_locked(id, approval, verified, source);
        let unlock = fs::flock(&self.dir, FlockOperation::Unlock).map_err(os_error);
        unlock?;
        result
    }
}
