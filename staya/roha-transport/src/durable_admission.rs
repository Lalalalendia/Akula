//! P2P-BLOB-07: restart-safe *reservation* bookkeeping for an isolated Linux lab.
//!
//! This is deliberately NOT an FsStore quota or an Akula File Gate permit.
//! The owner supplies a private (0700) metadata directory separate from data.
//! One OS advisory-locked owner writes a checksummed, bounded, fsync+renamed
//! reservation snapshot. Existing reservations ALWAYS survive owner death.
//! An orphan snapshot, suspicious directory entry, damaged state or uncertain
//! transaction MUST block admission rather than silently free disk capacity.
//!
//! The caller MUST bind a trusted VaultId / RevisionId / DeviceId / EndpointId
//! and exact BLAKE3+length BEFORE invoking reserve. This lab only stores the
//! transport peer key, hash and expected length. No real grants are made.
//! Filesystem, database and Bao overhead are NOT charged; before deployment,
//! a physical quota/cgroup/limited staging filesystem remains mandatory.

#![cfg(target_os = "linux")]

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use iroh::EndpointId;

use crate::admission::AdmissionLimits;

const MAGIC: &[u8; 8] = b"AKSTG07\0";
const FORMAT_VERSION: u16 = 1;
const MAX_RECORDS: usize = 256;
const HEADER_LEN: usize = 8 + 2 + 8 + 4 + 5 * 8;
const ENTRY_LEN: usize = 32 + 32 + 8 + 1;
const CHECKSUM_LEN: usize = 32;
const MAX_SNAPSHOT_BYTES: usize =
    HEADER_LEN + MAX_RECORDS * ENTRY_LEN + CHECKSUM_LEN;
const OWNER_FILE: &str = "owner.lock";
const SNAPSHOT_FILE: &str = "reservations.v1";
const NEXT_FILE: &str = "reservations.next";

#[derive(Debug)]
pub enum DurableError {
    Io(io::Error),
    InvalidPolicy,
    UnsafeDirectory,
    CompetingOwner,
    CorruptOrUnknownSnapshot,
    InterruptedTransaction,
    PolicyChanged,
    ObjectTooLarge,
    AlreadyReserved,
    SlotsExhausted,
    BytesExhausted,
    PeerSlotsExhausted,
    PeerBytesExhausted,
    NotFound,
    VerificationFailed,
    PoisonedWriter,
}

impl From<io::Error> for DurableError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DurableSnapshot {
    pub objects: usize,
    pub reserved_bytes: u64,
    pub verified_objects: usize,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Record {
    peer: [u8; 32],
    expected_bytes: u64,
    verified: bool,
}

/// A single owner of a private *metadata* directory. Keep alive while
/// admitting transfers. Reopening after SIGKILL recovers every persisted
/// reservation, including incomplete and verified-but-retained objects.
pub struct DurableLedger {
    root: PathBuf,
    _owner_lock: File,
    limits: AdmissionLimits,
    generation: u64,
    entries: BTreeMap<[u8; 32], Record>,
    poisoned: bool,
}

fn take<'a>(
    content: &'a [u8],
    cursor: &mut usize,
    size: usize,
) -> Result<&'a [u8], DurableError> {
    let end = cursor
        .checked_add(size)
        .ok_or(DurableError::CorruptOrUnknownSnapshot)?;
    let part = content
        .get(*cursor..end)
        .ok_or(DurableError::CorruptOrUnknownSnapshot)?;
    *cursor = end;
    Ok(part)
}

fn read_u64(content: &[u8], cursor: &mut usize) -> Result<u64, DurableError> {
    let bytes: [u8; 8] = take(content, cursor, 8)?
        .try_into()
        .map_err(|_| DurableError::CorruptOrUnknownSnapshot)?;
    Ok(u64::from_le_bytes(bytes))
}

fn policy_values(limits: AdmissionLimits) -> Result<[u64; 5], DurableError> {
    Ok([
        limits.max_object_bytes,
        limits.max_tracked_bytes,
        limits.max_peer_bytes,
        u64::try_from(limits.max_active).map_err(|_| DurableError::InvalidPolicy)?,
        u64::try_from(limits.max_peer_active).map_err(|_| DurableError::InvalidPolicy)?,
    ])
}

fn encode_snapshot(
    limits: AdmissionLimits,
    generation: u64,
    entries: &BTreeMap<[u8; 32], Record>,
) -> Result<Vec<u8>, DurableError> {
    if entries.len() > MAX_RECORDS {
        return Err(DurableError::SlotsExhausted);
    }
    let mut bytes = Vec::with_capacity(HEADER_LEN + entries.len() * ENTRY_LEN + CHECKSUM_LEN);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&generation.to_le_bytes());
    bytes.extend_from_slice(
        &u32::try_from(entries.len())
            .map_err(|_| DurableError::SlotsExhausted)?
            .to_le_bytes(),
    );
    for limit in policy_values(limits)? {
        bytes.extend_from_slice(&limit.to_le_bytes());
    }
    for (hash, record) in entries {
        bytes.extend_from_slice(hash);
        bytes.extend_from_slice(&record.peer);
        bytes.extend_from_slice(&record.expected_bytes.to_le_bytes());
        bytes.push(u8::from(record.verified));
    }
    bytes.extend_from_slice(blake3::hash(&bytes).as_bytes());
    Ok(bytes)
}

fn decode_snapshot(
    content: &[u8],
    limits: AdmissionLimits,
) -> Result<(u64, BTreeMap<[u8; 32], Record>), DurableError> {
    if content.len() < HEADER_LEN + CHECKSUM_LEN || content.len() > MAX_SNAPSHOT_BYTES {
        return Err(DurableError::CorruptOrUnknownSnapshot);
    }
    let (data, checksum) = content.split_at(content.len() - CHECKSUM_LEN);
    if blake3::hash(data).as_bytes().as_slice() != checksum {
        return Err(DurableError::CorruptOrUnknownSnapshot);
    }
    let mut pos = 0usize;
    if take(data, &mut pos, MAGIC.len())? != MAGIC.as_slice()
        || take(data, &mut pos, 2)? != FORMAT_VERSION.to_le_bytes().as_slice()
    {
        return Err(DurableError::CorruptOrUnknownSnapshot);
    }
    let generation = read_u64(data, &mut pos)?;
    let count_bytes: [u8; 4] = take(data, &mut pos, 4)?
        .try_into()
        .map_err(|_| DurableError::CorruptOrUnknownSnapshot)?;
    let count = u32::from_le_bytes(count_bytes) as usize;
    if count > MAX_RECORDS || count > limits.max_active
        || data.len() != HEADER_LEN + count * ENTRY_LEN
    {
        return Err(DurableError::CorruptOrUnknownSnapshot);
    }
    for expected in policy_values(limits)? {
        if read_u64(data, &mut pos)? != expected {
            return Err(DurableError::PolicyChanged);
        }
    }
    let mut entries = BTreeMap::new();
    let mut total = 0u64;
    let mut per_peer: BTreeMap<[u8; 32], (u64, usize)> = BTreeMap::new();
    for _ in 0..count {
        let hash: [u8; 32] = take(data, &mut pos, 32)?
            .try_into()
            .map_err(|_| DurableError::CorruptOrUnknownSnapshot)?;
        let peer: [u8; 32] = take(data, &mut pos, 32)?
            .try_into()
            .map_err(|_| DurableError::CorruptOrUnknownSnapshot)?;
        let expected_bytes = read_u64(data, &mut pos)?;
        let status = take(data, &mut pos, 1)?[0];
        if expected_bytes == 0 || expected_bytes > limits.max_object_bytes || status > 1 {
            return Err(DurableError::CorruptOrUnknownSnapshot);
        }
        total = total.checked_add(expected_bytes).ok_or(DurableError::CorruptOrUnknownSnapshot)?;
        let peer_total = per_peer.entry(peer).or_default();
        peer_total.0 = peer_total.0.checked_add(expected_bytes)
            .ok_or(DurableError::CorruptOrUnknownSnapshot)?;
        peer_total.1 = peer_total.1.checked_add(1)
            .ok_or(DurableError::CorruptOrUnknownSnapshot)?;
        if total > limits.max_tracked_bytes || peer_total.0 > limits.max_peer_bytes
            || peer_total.1 > limits.max_peer_active
            || entries.insert(hash, Record { peer, expected_bytes, verified: status == 1 }).is_some()
        {
            return Err(DurableError::CorruptOrUnknownSnapshot);
        }
    }
    if pos != data.len() {
        return Err(DurableError::CorruptOrUnknownSnapshot);
    }
    Ok((generation, entries))
}

fn check_metadata_directory(root: &Path) -> Result<(), DurableError> {
    let dir = fs::symlink_metadata(root)?;
    if !dir.file_type().is_dir() || dir.permissions().mode() & 0o077 != 0 {
        return Err(DurableError::UnsafeDirectory);
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().ok_or(DurableError::UnsafeDirectory)?;
        if !matches!(name, OWNER_FILE | SNAPSHOT_FILE | NEXT_FILE) {
            return Err(DurableError::UnsafeDirectory);
        }
        if !entry.file_type()?.is_file() {
            return Err(DurableError::UnsafeDirectory);
        }
    }
    Ok(())
}

fn sync_directory(root: &Path) -> Result<(), DurableError> {
    File::open(root)?.sync_all()?;
    Ok(())
}

impl DurableLedger {
    /// Caller provides an existing private owned directory (0700), separated
    /// from the FsStore data. Never create this at a user workspace root.
    pub fn open(root: &Path, limits: AdmissionLimits) -> Result<Self, DurableError> {
        limits.validate().map_err(|_| DurableError::InvalidPolicy)?;
        if limits.max_active > MAX_RECORDS {
            return Err(DurableError::InvalidPolicy);
        }
        check_metadata_directory(root)?;
        let lock_path = root.join(OWNER_FILE);
        // A prior owner marker with no snapshot is an interrupted or deleted
        // reservation ledger, NOT a fresh empty ledger. Refuse to reset it.
        let owner_preexisting = match fs::symlink_metadata(&lock_path) {
            Ok(_) => true,
            Err(err) if err.kind() == io::ErrorKind::NotFound => false,
            Err(err) => return Err(err.into()),
        };
        let owner = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .open(&lock_path)?;
        let lock_meta = fs::symlink_metadata(&lock_path)?;
        if !lock_meta.is_file() || lock_meta.nlink() != 1
            || lock_meta.permissions().mode() & 0o077 != 0
        {
            return Err(DurableError::UnsafeDirectory);
        }
        owner.try_lock().map_err(|_| DurableError::CompetingOwner)?;
        check_metadata_directory(root)?;
        if root.join(NEXT_FILE).exists() {
            return Err(DurableError::InterruptedTransaction);
        }
        let (generation, entries) = match fs::metadata(root.join(SNAPSHOT_FILE)) {
            Ok(meta) => {
                if !meta.is_file() || meta.len() > MAX_SNAPSHOT_BYTES as u64 {
                    return Err(DurableError::CorruptOrUnknownSnapshot);
                }
                decode_snapshot(&fs::read(root.join(SNAPSHOT_FILE))?, limits)?
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                if owner_preexisting {
                    return Err(DurableError::CorruptOrUnknownSnapshot);
                }
                let empty = BTreeMap::new();
                let bytes = encode_snapshot(limits, 0, &empty)?;
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(root.join(SNAPSHOT_FILE))?;
                file.write_all(&bytes)?;
                file.sync_all()?;
                sync_directory(root)?;
                (0, empty)
            }
            Err(err) => return Err(err.into()),
        };
        Ok(Self {
            root: root.to_owned(),
            _owner_lock: owner,
            limits,
            generation,
            entries,
            poisoned: false,
        })
    }

    pub fn snapshot(&self) -> Result<DurableSnapshot, DurableError> {
        if self.poisoned {
            return Err(DurableError::PoisonedWriter);
        }
        let reserved_bytes = self.entries.values().try_fold(0u64, |total, record| {
            total.checked_add(record.expected_bytes)
        }).ok_or(DurableError::CorruptOrUnknownSnapshot)?;
        Ok(DurableSnapshot {
            objects: self.entries.len(),
            reserved_bytes,
            verified_objects: self.entries.values().filter(|x| x.verified).count(),
            generation: self.generation,
        })
    }

    fn commit(&mut self, next: BTreeMap<[u8; 32], Record>) -> Result<(), DurableError> {
        if self.poisoned {
            return Err(DurableError::PoisonedWriter);
        }
        let generation = self.generation.checked_add(1)
            .ok_or(DurableError::CorruptOrUnknownSnapshot)?;
        let bytes = encode_snapshot(self.limits, generation, &next)?;
        let result = (|| -> Result<(), DurableError> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(self.root.join(NEXT_FILE))?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(self.root.join(NEXT_FILE), self.root.join(SNAPSHOT_FILE))?;
            sync_directory(&self.root)?;
            Ok(())
        })();
        if let Err(error) = result {
            self.poisoned = true;
            return Err(error);
        }
        self.entries = next;
        self.generation = generation;
        Ok(())
    }

    /// Reserve durable capacity BEFORE network and FS writer creation. The
    /// byte count and hash must be bound to trusted immutable revision data.
    pub fn reserve(
        &mut self,
        peer: EndpointId,
        hash: [u8; 32],
        expected_bytes: u64,
    ) -> Result<(), DurableError> {
        if self.poisoned {
            return Err(DurableError::PoisonedWriter);
        }
        if expected_bytes == 0 || expected_bytes > self.limits.max_object_bytes {
            return Err(DurableError::ObjectTooLarge);
        }
        if self.entries.contains_key(&hash) {
            return Err(DurableError::AlreadyReserved);
        }
        if self.entries.len() >= self.limits.max_active {
            return Err(DurableError::SlotsExhausted);
        }
        let peer_bytes = *peer.as_bytes();
        let mut current = 0u64;
        let mut peer_count = 0usize;
        let mut peer_total = 0u64;
        for record in self.entries.values() {
            current = current.checked_add(record.expected_bytes)
                .ok_or(DurableError::BytesExhausted)?;
            if record.peer == peer_bytes {
                peer_count += 1;
                peer_total = peer_total.checked_add(record.expected_bytes)
                    .ok_or(DurableError::PeerBytesExhausted)?;
            }
        }
        if peer_count >= self.limits.max_peer_active {
            return Err(DurableError::PeerSlotsExhausted);
        }
        if current.checked_add(expected_bytes)
            .filter(|sum| *sum <= self.limits.max_tracked_bytes)
            .is_none()
        {
            return Err(DurableError::BytesExhausted);
        }
        if peer_total.checked_add(expected_bytes)
            .filter(|sum| *sum <= self.limits.max_peer_bytes)
            .is_none()
        {
            return Err(DurableError::PeerBytesExhausted);
        }
        let mut next = self.entries.clone();
        next.insert(hash, Record { peer: peer_bytes, expected_bytes, verified: false });
        self.commit(next)
    }

    /// This marks the ledger verified only after independent exact-length
    /// BLAKE3 verification. It is NOT durable FsStore content validation or
    /// File Gate authority.
    pub fn verify_full(&mut self, hash: [u8; 32], bytes: &[u8]) -> Result<(), DurableError> {
        if self.poisoned {
            return Err(DurableError::PoisonedWriter);
        }
        let record = self.entries.get(&hash).ok_or(DurableError::NotFound)?;
        if usize::try_from(record.expected_bytes).ok() != Some(bytes.len())
            || blake3::hash(bytes).as_bytes() != &hash
        {
            return Err(DurableError::VerificationFailed);
        }
        let mut next = self.entries.clone();
        next.get_mut(&hash).ok_or(DurableError::NotFound)?.verified = true;
        self.commit(next)
    }

    /// Only invoke after a TRUSTED caller has removed all corresponding
    /// physical staging and synced its directory. The callback represents
    /// this external custody operation. On callback error, reservation stays.
    /// A malicious or incorrect callback cannot make this a hard disk quota.
    pub fn release_after_cleanup(
        &mut self,
        hash: [u8; 32],
        cleanup: impl FnOnce() -> io::Result<()>,
    ) -> Result<(), DurableError> {
        if self.poisoned {
            return Err(DurableError::PoisonedWriter);
        }
        if !self.entries.contains_key(&hash) {
            return Err(DurableError::NotFound);
        }
        cleanup()?;
        let mut next = self.entries.clone();
        next.remove(&hash);
        self.commit(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::{Command, Stdio},
        thread,
        time::Duration,
    };
    use iroh::SecretKey;

    fn limits() -> AdmissionLimits {
        AdmissionLimits {
            max_object_bytes: 1024,
            max_tracked_bytes: 1024,
            max_peer_bytes: 1024,
            max_active: 2,
            max_peer_active: 2,
        }
    }

    fn peer() -> EndpointId {
        SecretKey::generate().public()
    }

    fn digest(s: &[u8]) -> [u8; 32] {
        *blake3::hash(s).as_bytes()
    }

    fn temp_root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        // Never assume a particular tempfile version or runner umask gives
        // private permissions. The production gate REQUIRES mode 0700.
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        root
    }

    #[test]
    fn durable_reservation_survives_reopen_and_verification() {
        let dir = temp_root();
        let who = peer();
        let data = b"exact verified object";
        let key = digest(data);
        let generation;
        {
            let mut ledger = DurableLedger::open(dir.path(), limits()).unwrap();
            ledger.reserve(who, key, data.len() as u64).unwrap();
            ledger.verify_full(key, data).unwrap();
            generation = ledger.snapshot().unwrap().generation;
            assert_eq!(ledger.snapshot().unwrap().verified_objects, 1);
        }
        let mut reopened = DurableLedger::open(dir.path(), limits()).unwrap();
        assert_eq!(reopened.snapshot().unwrap().reserved_bytes, data.len() as u64);
        assert_eq!(reopened.snapshot().unwrap().verified_objects, 1);
        assert_eq!(reopened.snapshot().unwrap().generation, generation);
        let staging = dir.path().join("test-only-staged-object");
        fs::write(&staging, data).unwrap();
        reopened.release_after_cleanup(key, || {
            fs::remove_file(&staging)?;
            File::open(dir.path())?.sync_all()
        }).unwrap();
        assert_eq!(reopened.snapshot().unwrap().reserved_bytes, 0);
    }

    #[test]
    fn remaining_reservation_blocks_oversubscription_on_restart() {
        let dir = temp_root();
        let a = peer();
        let b = peer();
        {
            let mut ledger = DurableLedger::open(dir.path(), limits()).unwrap();
            ledger.reserve(a, digest(b"1"), 800).unwrap();
        }
        let mut recovered = DurableLedger::open(dir.path(), limits()).unwrap();
        assert!(matches!(
            recovered.reserve(b, digest(b"2"), 300),
            Err(DurableError::BytesExhausted)
        ));
        assert_eq!(recovered.snapshot().unwrap().reserved_bytes, 800);
        assert!(matches!(
            recovered.release_after_cleanup(digest(b"1"), || Err(io::Error::other("staging still exists"))),
            Err(DurableError::Io(_))
        ));
        assert_eq!(recovered.snapshot().unwrap().reserved_bytes, 800);
    }

    #[test]
    fn another_owner_cannot_open_while_first_holds_os_lock() {
        let dir = temp_root();
        let ledger = DurableLedger::open(dir.path(), limits()).unwrap();
        assert!(matches!(
            DurableLedger::open(dir.path(), limits()),
            Err(DurableError::CompetingOwner)
        ));
        drop(ledger);
        DurableLedger::open(dir.path(), limits()).unwrap();
    }

    #[test]
    fn corrupted_snapshots_and_orphan_write_are_fail_closed() {
        let dir = temp_root();
        {
            let mut ledger = DurableLedger::open(dir.path(), limits()).unwrap();
            ledger.reserve(peer(), digest(b"allocated"), 256).unwrap();
        }
        let path = dir.path().join(SNAPSHOT_FILE);
        let mut bytes = fs::read(&path).unwrap();
        bytes[HEADER_LEN + 5] ^= 0x01;
        fs::write(&path, bytes).unwrap();
        assert!(matches!(
            DurableLedger::open(dir.path(), limits()),
            Err(DurableError::CorruptOrUnknownSnapshot)
        ));
        fs::write(dir.path().join(NEXT_FILE), b"orphaned uncommitted transaction").unwrap();
        assert!(matches!(
            DurableLedger::open(dir.path(), limits()),
            Err(DurableError::InterruptedTransaction)
        ));
    }

    #[test]
    fn changed_limits_or_unknown_dir_entry_cannot_reset_accounting() {
        let dir = temp_root();
        DurableLedger::open(dir.path(), limits()).unwrap();
        let stricter = AdmissionLimits { max_tracked_bytes: 512, ..limits() };
        assert!(matches!(
            DurableLedger::open(dir.path(), stricter),
            Err(DurableError::InvalidPolicy)
        ));
        let changed = AdmissionLimits {
            max_object_bytes: 512,
            max_tracked_bytes: 1024,
            max_peer_bytes: 1024,
            ..limits()
        };
        assert!(matches!(
            DurableLedger::open(dir.path(), changed),
            Err(DurableError::PolicyChanged)
        ));
        fs::write(dir.path().join("untracked-file"), b"hidden stage").unwrap();
        assert!(matches!(
            DurableLedger::open(dir.path(), limits()),
            Err(DurableError::UnsafeDirectory)
        ));
    }

    #[test]
    fn missing_snapshot_with_existing_owner_marker_never_resets_quota() {
        let dir = temp_root();
        {
            let mut ledger = DurableLedger::open(dir.path(), limits()).unwrap();
            ledger.reserve(peer(), digest(b"cannot silently erase capacity"), 900).unwrap();
        }
        fs::remove_file(dir.path().join(SNAPSHOT_FILE)).unwrap();
        assert!(matches!(
            DurableLedger::open(dir.path(), limits()),
            Err(DurableError::CorruptOrUnknownSnapshot)
        ));
    }

    #[test]
    fn size_and_duplicate_denials_leave_snapshot_unchanged() {
        let dir = temp_root();
        let mut ledger = DurableLedger::open(dir.path(), limits()).unwrap();
        let who = peer();
        ledger.reserve(who, digest(b"one"), 800).unwrap();
        let snapshot = ledger.snapshot().unwrap();
        assert!(matches!(
            ledger.reserve(who, digest(b"oversized"), u64::MAX),
            Err(DurableError::ObjectTooLarge)
        ));
        assert!(matches!(
            ledger.reserve(who, digest(b"one"), 100),
            Err(DurableError::AlreadyReserved)
        ));
        assert!(matches!(
            ledger.reserve(who, digest(b"next"), 500),
            Err(DurableError::BytesExhausted)
        ));
        assert_eq!(ledger.snapshot().unwrap(), snapshot);
    }

    #[test]
    fn per_peer_byte_and_slot_limits_survive_restart() {
        let dir = temp_root();
        let policy = AdmissionLimits {
            max_object_bytes: 500,
            max_tracked_bytes: 1500,
            max_peer_bytes: 600,
            max_active: 3,
            max_peer_active: 2,
        };
        let a = peer();
        let b = peer();
        {
            let mut ledger = DurableLedger::open(dir.path(), policy).unwrap();
            ledger.reserve(a, digest(b"a1"), 500).unwrap();
            ledger.reserve(b, digest(b"b1"), 500).unwrap();
        }
        let mut recovered = DurableLedger::open(dir.path(), policy).unwrap();
        assert!(matches!(
            recovered.reserve(a, digest(b"a oversized"), 200),
            Err(DurableError::PeerBytesExhausted)
        ));
        recovered.reserve(a, digest(b"a2"), 100).unwrap();
        assert!(matches!(
            recovered.reserve(a, digest(b"a3"), 1),
            Err(DurableError::SlotsExhausted)
        ));
        assert_eq!(recovered.snapshot().unwrap().reserved_bytes, 1100);
    }

    /// Separate operating-system process must be refused while another
    /// process already holds the OS file lock; this helper is never run
    /// as a standalone normal unit test.
    #[test]
    #[ignore = "private competing-owner subprocess helper"]
    fn process_competing_owner_helper() {
        let root = PathBuf::from(std::env::var("AKULA_LEDGER_TEST_ROOT").unwrap());
        assert!(matches!(
            DurableLedger::open(&root, limits()),
            Err(DurableError::CompetingOwner)
        ));
    }

    #[test]
    fn simultaneously_opened_second_os_process_is_refused() {
        let dir = temp_root();
        let ledger = DurableLedger::open(dir.path(), limits()).unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("durable_admission::tests::process_competing_owner_helper")
            .arg("--ignored")
            .arg("--nocapture")
            .env("AKULA_LEDGER_TEST_ROOT", dir.path())
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "another process bypassed exclusive journal lock: {} {}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr),
        );
        drop(ledger);
    }

    /// Spawned solely by killed_owner_ledger_survives; not a standalone test.
    #[test]
    #[ignore = "private child OS process, killed by test parent"]
    fn process_crash_reservation_helper() {
        let root = PathBuf::from(std::env::var("AKULA_LEDGER_TEST_ROOT").unwrap());
        let mut ledger = DurableLedger::open(&root, limits()).unwrap();
        ledger.reserve(peer(), digest(b"after hard kill"), 900).unwrap();
        fs::write(root.join("ready.flag"), b"reserved and synced").unwrap();
        loop {
            thread::sleep(Duration::from_millis(100));
        }
    }

    #[test]
    fn killed_owner_ledger_survives_and_blocks_new_bytes() {
        let dir = temp_root();
        let executable = std::env::current_exe().unwrap();
        let mut child = Command::new(executable)
            .arg("--exact")
            .arg("durable_admission::tests::process_crash_reservation_helper")
            .arg("--ignored")
            .arg("--nocapture")
            .env("AKULA_LEDGER_TEST_ROOT", dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut ready = false;
        for _ in 0..300 {
            if dir.path().join("ready.flag").exists() {
                ready = true;
                break;
            }
            if let Some(status) = child.try_wait().unwrap() {
                panic!("child exited before SIGKILL: {status}");
            }
            thread::sleep(Duration::from_millis(10));
        }
        if !ready {
            let _ = child.kill();
            let _ = child.wait();
            panic!("child never durably reserved bytes");
        }
        child.kill().unwrap();
        let status = child.wait().unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(9));
        // The child marker is not staged content; keep metadata directory
        // strictly private and separate. No unknown entries may be admitted.
        fs::remove_file(dir.path().join("ready.flag")).unwrap();
        let mut reopened = DurableLedger::open(dir.path(), limits()).unwrap();
        assert_eq!(reopened.snapshot().unwrap().reserved_bytes, 900);
        assert!(matches!(
            reopened.reserve(peer(), digest(b"other object"), 200),
            Err(DurableError::BytesExhausted)
        ));
    }
}
