//! B1b1b0: opt-in Linux laboratory for a durable, operator-scoped release floor.
//!
//! Inert admission journal only: NEVER starts packages, edits systemd,
//! changes installed Akula, grants network access or authorizes file writes.
//! The caller is not allowed to supply the current rollback floor; it is
//! always loaded from this separately initialized private directory.
//!
//! This is NOT production root-owned or TPM-backed anti-rollback:
//! an attacker with this actor's UID/root access or an old disk snapshot
//! can replace/remove the whole directory. An independent OS-owned trust
//! anchor, physical power-loss tests and a pinned staged inode are future gates.

use crate::server_approval_lab::{
    verify_operator_approval, ApprovalContext, ApprovalError, SignedOperatorApproval,
    TrustedOperatorKey,
};
use crate::server_release_lab::{ReleaseFloor, Version};
use crate::server_staging_lab::InstallId;
use ring::digest;
use rustix::fs::{self, FlockOperation, Mode, OFlags, RenameFlags};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::Mutex;

const STATE_NAME: &str = "owner-floor.v1";
const PENDING_NAME: &str = "owner-floor.v1.pending";
const STATE_DOMAIN: &[u8] = b"AKULA-B1B1B0-OWNER-FLOOR-V1\0";
const STATE_HASH_DOMAIN: &str = "Akula B1b1b0 owner floor journal checksum v1";
const MAX_STATE_BYTES: u64 = 512;
const RECORD_BYTES: usize = 32 + 32 + 8 + 12 + 32 + 16 + 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerAnchor {
    pub host_key_sha256: [u8; 32],
    pub operator_public_key_sha256: [u8; 32],
}

impl OwnerAnchor {
    pub fn from_trusted_operator(host_key_sha256: [u8; 32], trusted: &TrustedOperatorKey) -> Self {
        let fingerprint = digest::digest(&digest::SHA256, &trusted.ed25519_public);
        Self {
            host_key_sha256,
            operator_public_key_sha256: fingerprint.as_ref().try_into()
                .expect("SHA256 is 32 bytes"),
        }
    }

    fn is_valid(self) -> bool {
        self.host_key_sha256.iter().any(|x| *x != 0)
            && self.operator_public_key_sha256.iter().any(|x| *x != 0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerFloorSnapshot {
    pub anchor: OwnerAnchor,
    pub floor: ReleaseFloor,
    pub last_install_id: InstallId,
    pub last_signed_approval_fingerprint: [u8; 32],
}

impl OwnerFloorSnapshot {
    fn initialized(anchor: OwnerAnchor) -> Self {
        Self {
            anchor,
            floor: ReleaseFloor::initial(),
            last_install_id: InstallId([0; 16]),
            last_signed_approval_fingerprint: [0; 32],
        }
    }

    fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(STATE_DOMAIN.len() + RECORD_BYTES + 32);
        data.extend_from_slice(STATE_DOMAIN);
        data.extend_from_slice(&self.anchor.host_key_sha256);
        data.extend_from_slice(&self.anchor.operator_public_key_sha256);
        data.extend_from_slice(&self.floor.sequence.to_be_bytes());
        data.extend_from_slice(&self.floor.version.major.to_be_bytes());
        data.extend_from_slice(&self.floor.version.minor.to_be_bytes());
        data.extend_from_slice(&self.floor.version.patch.to_be_bytes());
        data.extend_from_slice(&self.floor.signed_manifest_fingerprint);
        data.extend_from_slice(&self.last_install_id.0);
        data.extend_from_slice(&self.last_signed_approval_fingerprint);
        let mut hasher = blake3::Hasher::new_derive_key(STATE_HASH_DOMAIN);
        hasher.update(&data);
        data.extend_from_slice(hasher.finalize().as_bytes());
        data
    }

    fn deserialize(bytes: &[u8]) -> Result<Self, JournalError> {
        if bytes.len() != STATE_DOMAIN.len() + RECORD_BYTES + 32
            || !bytes.starts_with(STATE_DOMAIN)
        {
            return Err(JournalError::CorruptState);
        }
        let end = bytes.len() - 32;
        let mut hasher = blake3::Hasher::new_derive_key(STATE_HASH_DOMAIN);
        hasher.update(&bytes[..end]);
        if hasher.finalize().as_bytes() != &bytes[end..] {
            return Err(JournalError::CorruptState);
        }
        let mut pos = STATE_DOMAIN.len();
        let anchor = OwnerAnchor {
            host_key_sha256: read_fixed::<32>(bytes, &mut pos)?,
            operator_public_key_sha256: read_fixed::<32>(bytes, &mut pos)?,
        };
        let floor = ReleaseFloor {
            sequence: u64::from_be_bytes(read_fixed::<8>(bytes, &mut pos)?),
            version: Version {
                major: u32::from_be_bytes(read_fixed::<4>(bytes, &mut pos)?),
                minor: u32::from_be_bytes(read_fixed::<4>(bytes, &mut pos)?),
                patch: u32::from_be_bytes(read_fixed::<4>(bytes, &mut pos)?),
            },
            signed_manifest_fingerprint: read_fixed::<32>(bytes, &mut pos)?,
        };
        let last_install_id = InstallId(read_fixed::<16>(bytes, &mut pos)?);
        let last_signed_approval_fingerprint = read_fixed::<32>(bytes, &mut pos)?;
        if !anchor.is_valid() || pos != end {
            return Err(JournalError::CorruptState);
        }
        if floor.sequence == 0 {
            if floor != ReleaseFloor::initial()
                || last_install_id.0.iter().any(|b| *b != 0)
                || last_signed_approval_fingerprint.iter().any(|b| *b != 0)
            {
                return Err(JournalError::CorruptState);
            }
        } else if floor.signed_manifest_fingerprint.iter().all(|b| *b == 0)
            || last_install_id.0.iter().all(|b| *b == 0)
            || last_signed_approval_fingerprint.iter().all(|b| *b == 0)
        {
            return Err(JournalError::CorruptState);
        }
        Ok(Self { anchor, floor, last_install_id, last_signed_approval_fingerprint })
    }
}

fn read_fixed<const N: usize>(src: &[u8], pos: &mut usize)
    -> Result<[u8; N], JournalError>
{
    let end = pos.checked_add(N).ok_or(JournalError::CorruptState)?;
    let bytes: [u8; N] = src.get(*pos..end)
        .ok_or(JournalError::CorruptState)?
        .try_into().map_err(|_| JournalError::CorruptState)?;
    *pos = end;
    Ok(bytes)
}

#[derive(Debug)]
pub enum JournalError {
    Io(io::Error),
    Approval(ApprovalError),
    UnsafeDirectory,
    UnsafeEntry,
    InvalidAnchor,
    AlreadyInitialized,
    Uninitialized,
    NeedsRepair,
    CorruptState,
    AnchorMismatch,
    ReplayRefused,
    PoisonedLock,
    InjectedInterruption,
}

impl From<io::Error> for JournalError {
    fn from(value: io::Error) -> Self { Self::Io(value) }
}
impl From<ApprovalError> for JournalError {
    fn from(value: ApprovalError) -> Self { Self::Approval(value) }
}
fn os_error(err: rustix::io::Errno) -> JournalError {
    JournalError::Io(err.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InertCommitFault {
    None,
    AfterPendingSync,
    AfterRenameBeforeDirectorySync,
}

pub struct OwnerFloorJournalLab {
    dir: File,
    local_lock: Mutex<()>,
}

impl OwnerFloorJournalLab {
    /// The caller must supply a separately created, trusted parent and 0700 dir.
    /// Never creates a missing directory or automatically resets old state.
    pub fn open(path: &Path) -> Result<Self, JournalError> {
        let dir = File::from(fs::open(
            path, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ).map_err(os_error)?);
        let journal = Self { dir, local_lock: Mutex::new(()) };
        journal.validate_dir()?;
        Ok(journal)
    }

    fn validate_dir(&self) -> Result<(), JournalError> {
        let meta = self.dir.metadata()?;
        if !meta.is_dir() || meta.nlink() == 0
            || meta.uid() != rustix::process::geteuid().as_raw()
            || meta.permissions().mode() & 0o777 != 0o700
        {
            return Err(JournalError::UnsafeDirectory);
        }
        Ok(())
    }

    fn with_lock<T>(&self, f: impl FnOnce(&Self) -> Result<T, JournalError>)
        -> Result<T, JournalError>
    {
        let _local = self.local_lock.lock().map_err(|_| JournalError::PoisonedLock)?;
        fs::flock(&self.dir, FlockOperation::LockExclusive).map_err(os_error)?;
        let result = (|| { self.validate_dir()?; f(self) })();
        let unlock = fs::flock(&self.dir, FlockOperation::Unlock).map_err(os_error);
        unlock?;
        result
    }

    fn open_entry(&self, name: &str) -> Result<Option<File>, JournalError> {
        match fs::openat(
            &self.dir, name, OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => Ok(Some(File::from(fd))),
            Err(rustix::io::Errno::NOENT) => Ok(None),
            Err(_) => Err(JournalError::UnsafeEntry),
        }
    }

    fn exists(&self, name: &str) -> Result<bool, JournalError> {
        Ok(self.open_entry(name)?.is_some())
    }

    fn read_locked(&self) -> Result<OwnerFloorSnapshot, JournalError> {
        if self.exists(PENDING_NAME)? {
            return Err(JournalError::NeedsRepair);
        }
        let mut file = self.open_entry(STATE_NAME)?.ok_or(JournalError::Uninitialized)?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.nlink() != 1
            || meta.uid() != rustix::process::geteuid().as_raw()
            || meta.permissions().mode() & 0o777 != 0o600
        {
            return Err(JournalError::UnsafeEntry);
        }
        let mut bytes = Vec::new();
        (&mut file).take(MAX_STATE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(JournalError::CorruptState);
        }
        OwnerFloorSnapshot::deserialize(&bytes)
    }

    /// Dedicated test seam. A returned injected failure is NOT a commit receipt.
    fn persist_locked(
        &self, state: &OwnerFloorSnapshot, first: bool, fault: InertCommitFault,
    ) -> Result<(), JournalError> {
        if self.exists(PENDING_NAME)? {
            return Err(JournalError::NeedsRepair);
        }
        let mut file = File::from(fs::openat(
            &self.dir, PENDING_NAME,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        ).map_err(os_error)?);
        file.write_all(&state.serialize())?;
        file.sync_all()?;
        self.dir.sync_all()?; // durable intent before changing the accepted state
        if fault == InertCommitFault::AfterPendingSync {
            return Err(JournalError::InjectedInterruption); // pending is not cleaned
        }
        if first {
            fs::renameat_with(
                &self.dir, PENDING_NAME, &self.dir, STATE_NAME, RenameFlags::NOREPLACE,
            ).map_err(os_error)?;
        } else {
            fs::renameat(&self.dir, PENDING_NAME, &self.dir, STATE_NAME)
                .map_err(os_error)?;
        }
        if fault == InertCommitFault::AfterRenameBeforeDirectorySync {
            return Err(JournalError::InjectedInterruption);
        }
        self.dir.sync_all()?;
        Ok(())
    }

    /// Explicit lab-only initialization from independently trusted host and
    /// operator anchors. A vanished floor is NEVER initialized on ordinary open.
    pub fn initialize(&self, anchor: OwnerAnchor) -> Result<OwnerFloorSnapshot, JournalError> {
        self.with_lock(|this| {
            if !anchor.is_valid() { return Err(JournalError::InvalidAnchor); }
            if this.exists(PENDING_NAME)? { return Err(JournalError::NeedsRepair); }
            if this.exists(STATE_NAME)? { return Err(JournalError::AlreadyInitialized); }
            let state = OwnerFloorSnapshot::initialized(anchor);
            this.persist_locked(&state, true, InertCommitFault::None)?;
            Ok(state)
        })
    }

    pub fn snapshot(&self) -> Result<OwnerFloorSnapshot, JournalError> {
        self.with_lock(Self::read_locked)
    }

    /// An opt-in, INERT record of owner consent (not permission to execute).
    /// The rollback floor is always read under the journal lock, not supplied
    /// by a candidate or caller. A replay never returns a fresh success.
    pub fn record(
        &self, signed: &SignedOperatorApproval, trusted: &TrustedOperatorKey,
        context: ApprovalContext<'_>, now_unix: u64,
    ) -> Result<OwnerFloorSnapshot, JournalError> {
        self.record_with_fault(signed, trusted, context, now_unix, InertCommitFault::None)
    }

    #[doc(hidden)]
    pub fn record_with_fault(
        &self, signed: &SignedOperatorApproval, trusted: &TrustedOperatorKey,
        mut context: ApprovalContext<'_>, now_unix: u64, fault: InertCommitFault,
    ) -> Result<OwnerFloorSnapshot, JournalError> {
        self.with_lock(|this| {
            let previous = this.read_locked()?;
            let trusted_key_sha = digest::digest(&digest::SHA256, &trusted.ed25519_public);
            if previous.anchor.host_key_sha256 != context.observed_host_key_sha256
                || previous.anchor.operator_public_key_sha256 != trusted_key_sha.as_ref()
            {
                return Err(JournalError::AnchorMismatch);
            }
            // Do not trust a caller-supplied floor. Rebind the signature
            // verification to the current, locked and validated on-disk floor.
            context.persisted_floor = previous.floor;
            let verified = verify_operator_approval(signed, trusted, context, now_unix)?;
            if verified.proposed_floor.sequence <= previous.floor.sequence
                || verified.install_id == previous.last_install_id
                || verified.signed_approval_fingerprint
                    == previous.last_signed_approval_fingerprint
            {
                return Err(JournalError::ReplayRefused);
            }
            let next = OwnerFloorSnapshot {
                anchor: previous.anchor,
                floor: verified.proposed_floor,
                last_install_id: verified.install_id,
                last_signed_approval_fingerprint: verified.signed_approval_fingerprint,
            };
            this.persist_locked(&next, false, fault)?;
            Ok(next)
        })
    }
}
