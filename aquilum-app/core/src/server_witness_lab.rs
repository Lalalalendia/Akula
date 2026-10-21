//! B1b1c0: inert local witness-ledger laboratory, Linux only.
//!
//! Security boundary:
//! - Re-verifies the DETACHED owner approval signature inside the witness API.
//!   A caller cannot commit by constructing InertWitnessCasRequest directly.
//! - Writes a durable, append-only, checksummed record using directory-fd
//!   relative O_NOFOLLOW/O_EXCL, sync_all(file), NOREPLACE rename and
//!   sync_all(directory), with flock for cooperating independent processes.
//! - Every accepted revision consumes exactly one signed nonce and InstallId.
//!   A lost acknowledgment may report AlreadyRecorded, never a new grant.
//! - An independent trusted checkpoint MUST be supplied by a provisioner.
//!   This directory and hash chain are not that external authority.
//!
//! THIS IS NOT a privileged/root-owned witness, hostile-writer isolation,
//! kernel-enforced anti-rollback, actual UI consent, trusted clock, installer,
//! systemd activation, remote SSH transfer, or production acceptance.
//! Even a successful write cannot authorize execution or change a Node.

use crate::server_owner_approval_lab::{
    verify_owner_approval, ApprovalError, ExternalApprovalScope,
    InertWitnessCasRequest, SignedOwnerApproval, TrustedOwnerApprover,
};
use crate::server_release_floor_lab::{ProposedFloor, TrustedFloorHead};
use crate::server_release_lab::{ReleaseFloor, Version};
use crate::server_staging_lab::{ApprovedPlanDigest, InstallId, StageReceipt};
use rustix::fs::{self, FlockOperation, Mode, OFlags, RenameFlags};
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::Mutex;

const DOMAIN: &[u8] = b"AKULA-B1B1C0-INERT-WITNESS-RECORD-V1\0";
const HASH_DOMAIN: &str = "Akula B1b1c0 inert witness record checksum v1";
const CLAIM_HASH_DOMAIN: &str = "Akula B1b1c0 detached claim identity v1";
const HEAD_BYTES: usize = 8 + 8 + 3 * 4 + 32 + 32;
const BODY_BYTES: usize = DOMAIN.len() + 1 + 8 + 32
    + 2 * HEAD_BYTES + 16 + 32 + 32 + 32 + 16 + 32;
const RECORD_BYTES: usize = BODY_BYTES + 32;
const MAX_REVISIONS: u64 = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WitnessCheckpoint {
    pub revision: u64,
    pub accepted_floor: TrustedFloorHead,
    pub record_tip: [u8; 32],
}

impl WitnessCheckpoint {
    /// This sentinel is valid ONLY when a trusted external provisioner has
    /// explicitly approved first-time enrollment; missing files do not prove it.
    pub const fn genesis() -> Self {
        Self {
            revision: 0,
            accepted_floor: TrustedFloorHead::genesis(),
            record_tip: [0; 32],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WitnessDecision {
    /// A new inert record was persisted. NOT an activation grant.
    NewlyRecorded { checkpoint: WitnessCheckpoint },
    /// Lost-ACK exact replay. No write and NEVER a second grant.
    AlreadyRecorded { checkpoint: WitnessCheckpoint },
}

impl WitnessDecision {
    pub const fn requires_separate_activation_authority(&self) -> bool { true }
}

#[derive(Debug)]
pub enum WitnessError {
    Io(io::Error),
    Approval(ApprovalError),
    UnsafeDirectory,
    InvalidNamespace,
    UntrustedCheckpoint,
    CorruptRecord,
    NeedsRepair,
    CasConflict,
    ConsumedNonce,
    ReusedInstallId,
    Exhausted,
    PoisonedLock,
}

impl From<io::Error> for WitnessError {
    fn from(error: io::Error) -> Self { Self::Io(error) }
}
impl From<ApprovalError> for WitnessError {
    fn from(error: ApprovalError) -> Self { Self::Approval(error) }
}
fn errno(error: rustix::io::Errno) -> WitnessError {
    WitnessError::Io(error.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WitnessRecord {
    revision: u64,
    previous_record_tip: [u8; 32],
    previous_floor: TrustedFloorHead,
    proposed_floor: TrustedFloorHead,
    install_id: InstallId,
    plan_digest: ApprovedPlanDigest,
    host_key_digest: [u8; 32],
    package_digest: [u8; 32],
    nonce: [u8; 16],
    signed_claim_digest: [u8; 32],
    record_tip: [u8; 32],
}

fn claim_digest(signed: &SignedOwnerApproval) -> [u8; 32] {
    let mut hash = blake3::Hasher::new_derive_key(CLAIM_HASH_DOMAIN);
    hash.update(&signed.claim.canonical_bytes());
    hash.update(&signed.signature);
    *hash.finalize().as_bytes()
}

fn filename(revision: u64, pending: bool) -> String {
    format!("witness-{revision:016x}.{}",
        if pending { "pending" } else { "record" })
}

fn push_head(out: &mut Vec<u8>, head: TrustedFloorHead) {
    out.extend_from_slice(&head.generation.to_be_bytes());
    out.extend_from_slice(&head.floor.sequence.to_be_bytes());
    for part in [head.floor.version.major, head.floor.version.minor,
        head.floor.version.patch]
    {
        out.extend_from_slice(&part.to_be_bytes());
    }
    out.extend_from_slice(&head.floor.signed_manifest_fingerprint);
    out.extend_from_slice(&head.chain_tip);
}

fn encode_record(mut record: WitnessRecord) -> Result<(Vec<u8>, WitnessRecord), WitnessError> {
    let mut body = Vec::with_capacity(RECORD_BYTES);
    body.extend_from_slice(DOMAIN);
    body.push(1);
    body.extend_from_slice(&record.revision.to_be_bytes());
    body.extend_from_slice(&record.previous_record_tip);
    push_head(&mut body, record.previous_floor);
    push_head(&mut body, record.proposed_floor);
    body.extend_from_slice(&record.install_id.0);
    body.extend_from_slice(&record.plan_digest.0);
    body.extend_from_slice(&record.host_key_digest);
    body.extend_from_slice(&record.package_digest);
    body.extend_from_slice(&record.nonce);
    body.extend_from_slice(&record.signed_claim_digest);
    if body.len() != BODY_BYTES { return Err(WitnessError::CorruptRecord); }
    let mut hash = blake3::Hasher::new_derive_key(HASH_DOMAIN);
    hash.update(&body);
    record.record_tip = *hash.finalize().as_bytes();
    body.extend_from_slice(&record.record_tip);
    Ok((body, record))
}

struct Decoder<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl Decoder<'_> {
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N], WitnessError> {
        let end = self.pos.checked_add(N).ok_or(WitnessError::CorruptRecord)?;
        let range = self.bytes.get(self.pos..end).ok_or(WitnessError::CorruptRecord)?;
        self.pos = end;
        range.try_into().map_err(|_| WitnessError::CorruptRecord)
    }
    fn u64(&mut self) -> Result<u64, WitnessError> {
        Ok(u64::from_be_bytes(self.bytes::<8>()?))
    }
    fn u32(&mut self) -> Result<u32, WitnessError> {
        Ok(u32::from_be_bytes(self.bytes::<4>()?))
    }
    fn head(&mut self) -> Result<TrustedFloorHead, WitnessError> {
        let generation = self.u64()?;
        let sequence = self.u64()?;
        let version = Version {
            major: self.u32()?,
            minor: self.u32()?,
            patch: self.u32()?,
        };
        let fingerprint = self.bytes::<32>()?;
        let chain_tip = self.bytes::<32>()?;
        Ok(TrustedFloorHead {
            generation,
            floor: ReleaseFloor {
                sequence, version, signed_manifest_fingerprint: fingerprint,
            },
            chain_tip,
        })
    }
}

fn decode_record(bytes: &[u8]) -> Result<WitnessRecord, WitnessError> {
    if bytes.len() != RECORD_BYTES || !bytes.starts_with(DOMAIN)
        || bytes[DOMAIN.len()] != 1
    {
        return Err(WitnessError::CorruptRecord);
    }
    let mut hash = blake3::Hasher::new_derive_key(HASH_DOMAIN);
    hash.update(&bytes[..BODY_BYTES]);
    let checksum: [u8; 32] = *hash.finalize().as_bytes();
    if checksum.as_slice() != &bytes[BODY_BYTES..] {
        return Err(WitnessError::CorruptRecord);
    }
    let mut decoder = Decoder { bytes, pos: DOMAIN.len() + 1 };
    let revision = decoder.u64()?;
    let previous_record_tip = decoder.bytes::<32>()?;
    let previous_floor = decoder.head()?;
    let proposed_floor = decoder.head()?;
    let install_id = InstallId(decoder.bytes::<16>()?);
    let plan_digest = ApprovedPlanDigest(decoder.bytes::<32>()?);
    let host_key_digest = decoder.bytes::<32>()?;
    let package_digest = decoder.bytes::<32>()?;
    let nonce = decoder.bytes::<16>()?;
    let signed_claim_digest = decoder.bytes::<32>()?;
    if decoder.pos != BODY_BYTES || revision == 0
        || proposed_floor.generation != revision
        || previous_floor.generation.checked_add(1) != Some(revision)
        || proposed_floor.floor.sequence <= previous_floor.floor.sequence
        || proposed_floor.floor.version < previous_floor.floor.version
        || install_id.0 == [0; 16] || plan_digest.0 == [0; 32]
        || host_key_digest == [0; 32] || package_digest == [0; 32]
        || nonce == [0; 16] || signed_claim_digest == [0; 32]
    {
        return Err(WitnessError::CorruptRecord);
    }
    Ok(WitnessRecord {
        revision, previous_record_tip, previous_floor, proposed_floor,
        install_id, plan_digest, host_key_digest, package_digest, nonce,
        signed_claim_digest, record_tip: checksum,
    })
}

struct Scan {
    checkpoint: WitnessCheckpoint,
    last: Option<WitnessRecord>,
    nonces: HashSet<[u8; 16]>,
    install_ids: HashSet<[u8; 16]>,
}

pub struct InertWitnessLedgerLab {
    dir: File,
    local: Mutex<()>,
}

impl InertWitnessLedgerLab {
    /// Path and trusted ancestors must be controlled by an OUTSIDE authority.
    /// 0700+flock is only a cooperating-writer laboratory safety boundary.
    pub fn open(path: &Path) -> Result<Self, WitnessError> {
        let dir = File::from(fs::open(
            path, OFlags::RDONLY | OFlags::DIRECTORY |
                OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ).map_err(errno)?);
        let meta = dir.metadata()?;
        if !meta.is_dir() || meta.nlink() == 0
            || (meta.permissions().mode() & 0o777) != 0o700
            || meta.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(WitnessError::UnsafeDirectory);
        }
        Ok(Self { dir, local: Mutex::new(()) })
    }

    fn read_record(&self, revision: u64) -> Result<Option<WitnessRecord>, WitnessError> {
        let name = filename(revision, false);
        let mut file = match fs::openat(
            &self.dir, name.as_str(),
            OFlags::RDONLY | OFlags::NOFOLLOW |
                OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => File::from(fd),
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(error) => return Err(errno(error)),
        };
        let meta = file.metadata()?;
        if !meta.is_file() || meta.nlink() != 1
            || meta.uid() != rustix::process::geteuid().as_raw()
            || (meta.permissions().mode() & 0o777) != 0o600
            || meta.len() != RECORD_BYTES as u64
        {
            return Err(WitnessError::CorruptRecord);
        }
        let mut bytes = Vec::with_capacity(RECORD_BYTES);
        (&mut file).take((RECORD_BYTES + 1) as u64).read_to_end(&mut bytes)?;
        let record = decode_record(&bytes)?;
        if record.revision != revision { return Err(WitnessError::CorruptRecord); }
        Ok(Some(record))
    }

    fn pending_exists(&self, revision: u64) -> Result<bool, WitnessError> {
        let name = filename(revision, true);
        match fs::openat(
            &self.dir, name.as_str(),
            OFlags::RDONLY | OFlags::NONBLOCK |
                OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(_fd) => Ok(true),
            Err(rustix::io::Errno::NOENT) => Ok(false),
            Err(_) => Err(WitnessError::NeedsRepair),
        }
    }

    fn scan_locked(&self) -> Result<Scan, WitnessError> {
        let mut checkpoint = WitnessCheckpoint::genesis();
        let mut last: Option<WitnessRecord> = None;
        let mut nonces = HashSet::new();
        let mut install_ids = HashSet::new();
        for revision in 1..=MAX_REVISIONS {
            if self.pending_exists(revision)? {
                return Err(WitnessError::NeedsRepair);
            }
            let Some(record) = self.read_record(revision)? else {
                if self.read_record(revision + 1)?.is_some()
                    || self.pending_exists(revision + 1)?
                {
                    return Err(WitnessError::NeedsRepair);
                }
                return Ok(Scan { checkpoint, last, nonces, install_ids });
            };
            if record.previous_record_tip != checkpoint.record_tip
                || record.previous_floor != checkpoint.accepted_floor
                || !nonces.insert(record.nonce)
                || !install_ids.insert(record.install_id.0)
            {
                return Err(WitnessError::CorruptRecord);
            }
            checkpoint = WitnessCheckpoint {
                revision,
                accepted_floor: record.proposed_floor,
                record_tip: record.record_tip,
            };
            last = Some(record);
        }
        Err(WitnessError::Exhausted)
    }

    fn record_locked(
        &self,
        request: InertWitnessCasRequest,
        scope: ExternalApprovalScope,
        signed: &SignedOwnerApproval,
        staged: &StageReceipt,
        independently_pinned: WitnessCheckpoint,
    ) -> Result<WitnessDecision, WitnessError> {
        // NEVER infer genesis from an empty directory. The caller must pin
        // an external checkpoint provisioned outside candidate package/SSH.
        if independently_pinned.revision != request.expected_revision
            || independently_pinned.accepted_floor != request.previous_head
            || (independently_pinned.revision == 0
                && independently_pinned != WitnessCheckpoint::genesis())
            || request.desired_revision > MAX_REVISIONS
            || request.proposed_head.floor != staged.verified_package.proposed_floor
        {
            return Err(WitnessError::UntrustedCheckpoint);
        }
        let mut scan = self.scan_locked()?;
        let candidate = WitnessRecord {
            revision: request.desired_revision,
            previous_record_tip: independently_pinned.record_tip,
            previous_floor: request.previous_head,
            proposed_floor: request.proposed_head,
            install_id: request.install_id,
            plan_digest: request.approved_plan_digest,
            host_key_digest: scope.pinned_host_key_sha256,
            package_digest: staged.verified_package.package_sha256,
            nonce: request.approval_nonce,
            signed_claim_digest: claim_digest(signed),
            record_tip: [0; 32],
        };
        let (bytes, expected_record) = encode_record(candidate)?;

        // An exact retry after a lost ACK may report that an inert record is
        // ALREADY PRESENT, but never issues another installation capability.
        if scan.checkpoint.revision == request.desired_revision
            && scan.last.as_ref() == Some(&expected_record)
            && independently_pinned.revision + 1 == request.desired_revision
        {
            return Ok(WitnessDecision::AlreadyRecorded {
                checkpoint: scan.checkpoint,
            });
        }
        // The FIRST question for a different request is whether this caller
        // still holds the current independently anchored revision. A stale
        // request is a CAS conflict, not an attempt to consume a second nonce
        // or a different InstallId. Exact lost-ACK retry was handled above.
        if scan.checkpoint != independently_pinned
            || request.desired_revision != scan.checkpoint.revision + 1
        {
            return Err(WitnessError::CasConflict);
        }
        // Only a correctly sequenced new proposal can be checked against
        // historical nonce and InstallId consumption.
        if scan.nonces.contains(&request.approval_nonce) {
            return Err(WitnessError::ConsumedNonce);
        }
        if scan.install_ids.contains(&request.install_id.0) {
            return Err(WitnessError::ReusedInstallId);
        }
        if self.pending_exists(request.desired_revision)?
            || self.read_record(request.desired_revision)?.is_some()
        {
            return Err(WitnessError::NeedsRepair);
        }

        let pending = filename(request.desired_revision, true);
        let committed = filename(request.desired_revision, false);
        let mut file = File::from(fs::openat(
            &self.dir, pending.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL |
                OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        ).map_err(errno)?);
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::renameat_with(
            &self.dir, pending.as_str(), &self.dir, committed.as_str(),
            RenameFlags::NOREPLACE,
        ).map_err(errno)?;
        self.dir.sync_all()?;

        // Re-open by pinned directory FD; never authorize activation.
        let readback = self.read_record(request.desired_revision)?
            .ok_or(WitnessError::NeedsRepair)?;
        if readback != expected_record { return Err(WitnessError::CorruptRecord); }
        scan.checkpoint = WitnessCheckpoint {
            revision: request.desired_revision,
            accepted_floor: request.proposed_head,
            record_tip: expected_record.record_tip,
        };
        Ok(WitnessDecision::NewlyRecorded {
            checkpoint: scan.checkpoint,
        })
    }

    /// Consumes a validated detached approval into an INERT local journal.
    /// The caller MUST separately control signed-approval issuance and supply
    /// the independently trusted checkpoint; the returned decision does NOT
    /// authorize any installer/service operation.
    /// Read the locally durable checkpoint under the same cross-process
    /// flock used by CAS. This is NOT an external monotonic authority:
    /// restoring a filesystem snapshot can still roll the local state back.
    pub fn current_checkpoint(&self) -> Result<WitnessCheckpoint, WitnessError> {
        let _guard = self.local.lock().map_err(|_| WitnessError::PoisonedLock)?;
        fs::flock(&self.dir, FlockOperation::LockExclusive).map_err(errno)?;
        let scanned = self.scan_locked().map(|state| state.checkpoint);
        let unlock = fs::flock(&self.dir, FlockOperation::Unlock).map_err(errno);
        unlock?;
        scanned
    }

    pub fn record_signed_approval(
        &self,
        signed: &SignedOwnerApproval,
        trusted_approver: &TrustedOwnerApprover,
        scope: ExternalApprovalScope,
        staged: &StageReceipt,
        proposed: ProposedFloor,
        independently_pinned: WitnessCheckpoint,
        now_unix: u64,
    ) -> Result<WitnessDecision, WitnessError> {
        // Trust is re-established INSIDE this API, not in its caller.
        let request = verify_owner_approval(
            signed, trusted_approver, scope, staged, proposed, now_unix,
        )?;
        let _local = self.local.lock().map_err(|_| WitnessError::PoisonedLock)?;
        fs::flock(&self.dir, FlockOperation::LockExclusive).map_err(errno)?;
        let result = self.record_locked(
            request, scope, signed, staged, independently_pinned,
        );
        let unlock = fs::flock(&self.dir, FlockOperation::Unlock).map_err(errno);
        unlock?;
        result
    }
}
