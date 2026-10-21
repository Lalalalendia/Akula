//! B1b1a: Linux-only inert release-floor ledger laboratory.
//!
//! This is NOT a signed installer, trusted authorizer, activation gate or
//! protected OS authority. The independently trusted head MUST be obtained
//! from outside this directory; a caller passing a stale/forged genesis value
//! forfeits rollback protection. No systemd, SSH, elevated service, execution,
//! File Gate writes, Node identity or user data are accessed here.
//!
//! A caller first verifies a B1a signed manifest AND a B1b0 staged package,
//! then presents a synthetic immutable binding of InstallId, plan digest,
//! SSH-host-key digest and release identity. This binding is NOT consent.
//!
//! Each generation is written exclusively, synced, renamed with NOREPLACE
//! and directory-synced. A prior independently pinned chain tip is required.
//! The resulting proposal is NOT trusted until an independent owner-controlled
//! witness has durably accepted it. Crashes before rename leave a pending file
//! and force NeedsRepair. A crash after rename is idempotently recognizable.
//! No test here proves hardware power-loss durability or hostile-root safety.

use crate::server_release_lab::{
    ReleaseError, ReleaseFloor, VerifiedPackageReceipt, VerifiedRelease, Version,
};
use crate::server_staging_lab::{ApprovedPlanDigest, InstallId, StageReceipt};
use rustix::fs::{self, FlockOperation, Mode, OFlags, RenameFlags};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::Mutex;

const RECORD_DOMAIN: &[u8] = b"AKULA-B1B1A-FLOOR-RECORD-V1\0";
const RECORD_HASH_DOMAIN: &str = "Akula B1b1a untrusted local floor chain v1";
const RAW_RECORD_SIZE: usize =
    RECORD_DOMAIN.len() + 8 + 32 + 8 + 12 + 32 + 16 + 32 + 32 + 32 + 32 + 8;
const RECORD_SIZE: usize = RAW_RECORD_SIZE + 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrustedFloorHead {
    pub generation: u64,
    pub floor: ReleaseFloor,
    pub chain_tip: [u8; 32],
}

impl TrustedFloorHead {
    /// Genesis is allowed ONLY by a trusted external provisioner for a
    /// genuinely new ledger. Do not derive it from a missing on-disk file.
    pub const fn genesis() -> Self {
        Self {
            generation: 0,
            floor: ReleaseFloor::initial(),
            chain_tip: [0; 32],
        }
    }

    fn validate(&self) -> Result<(), LedgerError> {
        if self.generation == 0 {
            if *self != Self::genesis() {
                return Err(LedgerError::AnchorConflict);
            }
        } else if self.floor.sequence == 0
            || self.floor.signed_manifest_fingerprint == [0; 32]
            || self.chain_tip == [0; 32]
        {
            return Err(LedgerError::AnchorConflict);
        }
        Ok(())
    }
}

/// An inert typed binding supplied by an OUTSIDE authorization service.
/// Constructing this struct does not prove that anybody approved the plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LabPlanBinding {
    pub install_id: InstallId,
    pub plan_digest: ApprovedPlanDigest,
    pub pinned_host_key_sha256: [u8; 32],
    pub signed_manifest_fingerprint: [u8; 32],
}

impl LabPlanBinding {
    fn validate(&self) -> Result<(), LedgerError> {
        if self.install_id.0 == [0; 16]
            || self.plan_digest.0 == [0; 32]
            || self.pinned_host_key_sha256 == [0; 32]
            || self.signed_manifest_fingerprint == [0; 32]
        {
            return Err(LedgerError::InvalidBinding);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProposedFloor {
    /// Candidate, NOT trusted until an independent witness records it.
    pub proposed_head: TrustedFloorHead,
    pub idempotent_replay: bool,
    pub requires_external_witness: bool,
}

#[derive(Debug)]
pub enum LedgerError {
    Io(io::Error),
    Release(ReleaseError),
    UnsafeDirectory,
    CorruptRecord,
    NeedsRepair,
    AnchorConflict,
    InvalidBinding,
    PackageMismatch,
    RollbackRefused,
    Exhausted,
    PoisonedLock,
}

impl From<io::Error> for LedgerError {
    fn from(value: io::Error) -> Self { Self::Io(value) }
}

impl From<ReleaseError> for LedgerError {
    fn from(value: ReleaseError) -> Self { Self::Release(value) }
}

fn errno(error: rustix::io::Errno) -> LedgerError {
    LedgerError::Io(error.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LedgerRecord {
    head: TrustedFloorHead,
    previous_tip: [u8; 32],
    binding: LabPlanBinding,
    package_sha256: [u8; 32],
    package_size: u64,
}

fn name_for(generation: u64, pending: bool) -> String {
    format!("floor-{generation:016x}.{}", if pending { "pending" } else { "record" })
}

fn record_bytes(
    previous: TrustedFloorHead,
    binding: LabPlanBinding,
    receipt: &VerifiedPackageReceipt,
) -> Result<(Vec<u8>, TrustedFloorHead), LedgerError> {
    let generation = previous.generation.checked_add(1).ok_or(LedgerError::Exhausted)?;
    let floor = receipt.proposed_floor;
    let mut out = Vec::with_capacity(RECORD_SIZE);
    out.extend_from_slice(RECORD_DOMAIN);
    out.extend_from_slice(&generation.to_be_bytes());
    out.extend_from_slice(&previous.chain_tip);
    out.extend_from_slice(&floor.sequence.to_be_bytes());
    for v in [floor.version.major, floor.version.minor, floor.version.patch] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.extend_from_slice(&floor.signed_manifest_fingerprint);
    out.extend_from_slice(&binding.install_id.0);
    out.extend_from_slice(&binding.plan_digest.0);
    out.extend_from_slice(&binding.pinned_host_key_sha256);
    out.extend_from_slice(&binding.signed_manifest_fingerprint);
    out.extend_from_slice(&receipt.package_sha256);
    out.extend_from_slice(&receipt.byte_length.to_be_bytes());
    // Keep every record's checksum chained to the independently anchored tip.
    let mut hash = blake3::Hasher::new_derive_key(RECORD_HASH_DOMAIN);
    hash.update(&out);
    let tip: [u8; 32] = *hash.finalize().as_bytes();
    out.extend_from_slice(&tip);
    if out.len() != RECORD_SIZE {
        return Err(LedgerError::CorruptRecord);
    }
    Ok((out, TrustedFloorHead { generation, floor, chain_tip: tip }))
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], LedgerError> {
        let end = self.cursor.checked_add(N).ok_or(LedgerError::CorruptRecord)?;
        let slice = self.bytes.get(self.cursor..end).ok_or(LedgerError::CorruptRecord)?;
        self.cursor = end;
        slice.try_into().map_err(|_| LedgerError::CorruptRecord)
    }
    fn u64(&mut self) -> Result<u64, LedgerError> {
        Ok(u64::from_be_bytes(self.take::<8>()?))
    }
    fn u32(&mut self) -> Result<u32, LedgerError> {
        Ok(u32::from_be_bytes(self.take::<4>()?))
    }
}

fn decode_record(bytes: &[u8]) -> Result<LedgerRecord, LedgerError> {
    if bytes.len() != RECORD_SIZE || !bytes.starts_with(RECORD_DOMAIN) {
        return Err(LedgerError::CorruptRecord);
    }
    let mut hash = blake3::Hasher::new_derive_key(RECORD_HASH_DOMAIN);
    hash.update(&bytes[..RAW_RECORD_SIZE]);
    let observed = hash.finalize();
    if observed.as_bytes() != &bytes[RAW_RECORD_SIZE..] {
        return Err(LedgerError::CorruptRecord);
    }
    let mut reader = Reader { bytes, cursor: RECORD_DOMAIN.len() };
    let generation = reader.u64()?;
    let previous_tip = reader.take::<32>()?;
    let sequence = reader.u64()?;
    let version = Version {
        major: reader.u32()?,
        minor: reader.u32()?,
        patch: reader.u32()?,
    };
    let fingerprint = reader.take::<32>()?;
    let install_id = InstallId(reader.take::<16>()?);
    let plan_digest = ApprovedPlanDigest(reader.take::<32>()?);
    let pinned_host_key_sha256 = reader.take::<32>()?;
    let signed_manifest_fingerprint = reader.take::<32>()?;
    let package_sha256 = reader.take::<32>()?;
    let package_size = reader.u64()?;
    if reader.cursor != RAW_RECORD_SIZE || generation == 0 || sequence == 0 {
        return Err(LedgerError::CorruptRecord);
    }
    let head = TrustedFloorHead {
        generation,
        floor: ReleaseFloor {
            sequence,
            version,
            signed_manifest_fingerprint: fingerprint,
        },
        chain_tip: observed.as_bytes().to_owned(),
    };
    let binding = LabPlanBinding {
        install_id, plan_digest, pinned_host_key_sha256, signed_manifest_fingerprint,
    };
    binding.validate().map_err(|_| LedgerError::CorruptRecord)?;
    if fingerprint != signed_manifest_fingerprint || package_sha256 == [0; 32]
        || package_size == 0 {
        return Err(LedgerError::CorruptRecord);
    }
    Ok(LedgerRecord {
        head, previous_tip, binding, package_sha256, package_size,
    })
}

pub struct FloorLedgerLab {
    dir: File,
    local_lock: Mutex<()>,
}

impl FloorLedgerLab {
    /// Caller must supply a dedicated private directory, not the staging dir,
    /// whose path ancestors are trusted by a separate OS-owned installer.
    pub fn open(path: &Path) -> Result<Self, LedgerError> {
        let dir = File::from(fs::open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ).map_err(errno)?);
        let meta = dir.metadata()?;
        if !meta.is_dir() || meta.nlink() == 0
            || (meta.permissions().mode() & 0o777) != 0o700
            || meta.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(LedgerError::UnsafeDirectory);
        }
        Ok(Self { dir, local_lock: Mutex::new(()) })
    }

    fn read_record(&self, generation: u64) -> Result<Option<LedgerRecord>, LedgerError> {
        let name = name_for(generation, false);
        let fd = fs::openat(
            &self.dir, name.as_str(),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        );
        let mut file = match fd {
            Ok(fd) => File::from(fd),
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(e) => return Err(errno(e)),
        };
        let meta = file.metadata()?;
        if !meta.is_file() || meta.nlink() != 1
            || meta.uid() != rustix::process::geteuid().as_raw()
            || (meta.permissions().mode() & 0o777) != 0o600
            || meta.len() != RECORD_SIZE as u64
        {
            return Err(LedgerError::CorruptRecord);
        }
        let mut bytes = Vec::with_capacity(RECORD_SIZE);
        (&mut file).take((RECORD_SIZE + 1) as u64).read_to_end(&mut bytes)?;
        let record = decode_record(&bytes)?;
        if record.head.generation != generation {
            return Err(LedgerError::CorruptRecord);
        }
        Ok(Some(record))
    }

    fn pending_exists(&self, generation: u64) -> Result<bool, LedgerError> {
        let name = name_for(generation, true);
        match fs::openat(
            &self.dir, name.as_str(),
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(_fd) => Ok(true),
            Err(rustix::io::Errno::NOENT) => Ok(false),
            // A dangling symlink, permission failure or alien type must
            // fail closed rather than be interpreted as a missing transaction.
            Err(_) => Err(LedgerError::NeedsRepair),
        }
    }

    fn propose_locked(
        &self,
        expected: TrustedFloorHead,
        binding: LabPlanBinding,
        receipt: &VerifiedPackageReceipt,
    ) -> Result<ProposedFloor, LedgerError> {
        expected.validate()?;
        binding.validate()?;
        if expected.generation != 0 {
            let previous = self.read_record(expected.generation)?
                .ok_or(LedgerError::NeedsRepair)?;
            if previous.head != expected {
                return Err(LedgerError::AnchorConflict);
            }
        }
        let next = expected.generation.checked_add(1).ok_or(LedgerError::Exhausted)?;
        let after_next = next.checked_add(1).ok_or(LedgerError::Exhausted)?;
        if self.pending_exists(next)? || self.read_record(after_next)?.is_some() {
            return Err(LedgerError::NeedsRepair);
        }
        if receipt.proposed_floor.sequence <= expected.floor.sequence
            || receipt.proposed_floor.version < expected.floor.version
        {
            return Err(LedgerError::RollbackRefused);
        }
        if binding.signed_manifest_fingerprint
            != receipt.signed_manifest_fingerprint
        {
            return Err(LedgerError::InvalidBinding);
        }

        let (bytes, head) = record_bytes(expected, binding, receipt)?;
        if let Some(existing) = self.read_record(next)? {
            let proposed = decode_record(&bytes)?;
            let idempotent = existing == proposed && existing.previous_tip == expected.chain_tip;
            if !idempotent {
                return Err(LedgerError::AnchorConflict);
            }
            return Ok(ProposedFloor {
                proposed_head: head, idempotent_replay: true,
                requires_external_witness: true,
            });
        }

        let pending = name_for(next, true);
        let committed = name_for(next, false);
        let mut file = File::from(fs::openat(
            &self.dir, pending.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL
                | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        ).map_err(errno)?);
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::renameat_with(
            &self.dir, pending.as_str(), &self.dir, committed.as_str(),
            RenameFlags::NOREPLACE,
        ).map_err(errno)?;
        self.dir.sync_all()?;
        Ok(ProposedFloor {
            proposed_head: head, idempotent_replay: false,
            requires_external_witness: true,
        })
    }

    /// Never changes the durable independent witness or activates the Node.
    ///
    /// B1a VerifiedRelease is a Rust-typed signed-manifest admission. Rehash
    /// the complete bytes AGAIN; B1b0's receipt alone is not a proof of a
    /// still-pinned inode, real UI approval or stable host identity.
    pub fn propose<R: Read>(
        &self,
        expected: TrustedFloorHead,
        binding: LabPlanBinding,
        staged: &StageReceipt,
        verified: &VerifiedRelease,
        artifact: &mut R,
    ) -> Result<ProposedFloor, LedgerError> {
        if staged.install_id != binding.install_id
            || staged.approved_plan_digest != binding.plan_digest
        {
            return Err(LedgerError::InvalidBinding);
        }
        let receipt = verified.verify_package_stream(artifact)?;
        if receipt != staged.verified_package {
            return Err(LedgerError::PackageMismatch);
        }
        let _local = self.local_lock.lock().map_err(|_| LedgerError::PoisonedLock)?;
        fs::flock(&self.dir, FlockOperation::LockExclusive).map_err(errno)?;
        let result = self.propose_locked(expected, binding, &receipt);
        let unlock = fs::flock(&self.dir, FlockOperation::Unlock).map_err(errno);
        unlock?;
        result
    }
}
