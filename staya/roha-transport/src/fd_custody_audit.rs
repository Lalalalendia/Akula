//! P2P-BLOB-11a — Linux fd-relative, read-only FSStore custody lab.
//!
//! All child paths are resolved from an already pinned bounded mount descriptor
//! via openat2(BENEATH | NO_SYMLINKS | NO_XDEV). A candidate cannot redirect
//! us through a symlink, bind mount, or path traversal. Full content and Bao
//! proofs are rechecked on the SAME opened file descriptors, using the exact
//! BLAKE3 approved by the exclusively owned durable ledger.
//!
//! This is NOT a coherent atomic snapshot of redb + filesystem + journal:
//! redb's read-only lock excludes cooperating pinned redb writers but DOES NOT
//! stop an uncooperative process from overwriting an external .data or .obao4
//! inode or a privileged mount administrator. Such isolation remains HOLD.
//! No writable FsStore, automatic repair, quota release, or File Gate action.

#![cfg(target_os = "linux")]

use std::{
    fs::{File, Metadata},
    io::{self, Read},
    os::{
        fd::AsRawFd,
        unix::fs::{FileExt, MetadataExt},
    },
};

use bao_tree::{
    BaoTree, BlockSize, ChunkRanges,
    io::{outboard::PreOrderOutboard, sync::encode_ranges_validated},
};
use redb::{ReadOnlyDatabase, ReadableDatabase};
use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};

use crate::{
    bounded_staging::{StagingVolumeGuard, VolumeError},
    durable_admission::{DurableError, DurableLedger},
    joint_staging_audit::{JointHold, audit_quiescent_staging},
};

const BLOCK: BlockSize = BlockSize::from_chunk_log(4);
const MAX_DATA: u64 = 256 * 1024 * 1024;
const MAX_OUTBOARD: u64 = 4 * 1024 * 1024;
const MAX_DB: u64 = 64 * 1024 * 1024;

#[derive(Debug)]
pub enum FdHold {
    Volume(VolumeError),
    Ledger(DurableError),
    Joint(JointHold),
    Io(io::Error),
    InvalidChildName,
    UnsafeRelativePath,
    UnsafeStoreEntry,
    UnsafeDataFile,
    ChangedDirectoryBinding,
    ChangedFileBinding,
    ChangedOpenFile,
    DatabaseChanged,
    RedbReadOnlyUnavailable,
    UnverifiedOrUnknownReservation,
    InvalidObjectLength,
    WrongDataHash,
    NonCanonicalOutboard,
    InvalidBaoProof,
}

impl From<VolumeError> for FdHold {
    fn from(value: VolumeError) -> Self {
        Self::Volume(value)
    }
}
impl From<DurableError> for FdHold {
    fn from(value: DurableError) -> Self {
        Self::Ledger(value)
    }
}
impl From<JointHold> for FdHold {
    fn from(value: JointHold) -> Self {
        Self::Joint(value)
    }
}
impl From<io::Error> for FdHold {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

fn valid_component(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
                || b == b'.')
        && name != "."
        && name != ".."
        && !name.contains("..")
}

/// The kernel performs containment resolution during the open. No path is
/// reconstructed from the process CWD or from an untrusted database record.
/// ENOSYS/EAGAIN/EXDEV/ELOOP all fail closed; we never use unsafe open fallback.
fn child(parent: &File, name: &str, dir: bool) -> Result<File, FdHold> {
    if !valid_component(name) {
        return Err(FdHold::InvalidChildName);
    }
    let mut flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW;
    if dir {
        flags |= OFlags::DIRECTORY;
    }
    let resolve =
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV;
    let fd = openat2(parent, name, flags, Mode::empty(), resolve)
        .map_err(|_| FdHold::UnsafeRelativePath)?;
    Ok(File::from(fd))
}

fn check_dir(file: &File, mount_dev: u64) -> Result<(), FdHold> {
    let meta = file.metadata()?;
    if !meta.file_type().is_dir() || meta.dev() != mount_dev {
        return Err(FdHold::UnsafeStoreEntry);
    }
    Ok(())
}

fn check_regular(file: &File, mount_dev: u64, max: u64) -> Result<Metadata, FdHold> {
    let meta = file.metadata()?;
    if !meta.file_type().is_file()
        || meta.dev() != mount_dev
        || meta.nlink() != 1
        || meta.len() == 0
        || meta.len() > max
    {
        return Err(FdHold::UnsafeDataFile);
    }
    Ok(meta)
}

fn same_node(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev() && a.ino() == b.ino()
}

fn same_file(a: &Metadata, b: &Metadata) -> bool {
    same_node(a, b) && a.len() == b.len() && a.nlink() == b.nlink()
}

fn unchanged_in_place(a: &Metadata, b: &Metadata) -> bool {
    same_file(a, b)
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
}

/// Bounded digest on a pinned descriptor, using read_at to avoid changing
/// shared file offsets in a concurrently used redb/file handle.
fn digest_pinned(file: &File, size: u64, max: u64) -> Result<[u8; 32], FdHold> {
    if size == 0 || size > max {
        return Err(FdHold::InvalidObjectLength);
    }
    let mut hasher = blake3::Hasher::new();
    let mut block = [0u8; 64 * 1024];
    let mut offset = 0u64;
    while offset < size {
        let want = (size - offset).min(block.len() as u64) as usize;
        let n = file.read_at(&mut block[..want], offset)?;
        if n == 0 {
            return Err(FdHold::ChangedOpenFile);
        }
        hasher.update(&block[..n]);
        offset += n as u64;
    }
    let mut extra = [0u8; 1];
    if file.read_at(&mut extra, size)? != 0 {
        return Err(FdHold::ChangedOpenFile);
    }
    Ok(*hasher.finalize().as_bytes())
}

/// Stronger physical-name custody than the old path scanners, but still only
/// observational evidence (neither a file permission nor a writer fence).
pub struct PinnedFsStore<'a> {
    volume: &'a StagingVolumeGuard,
    store_child: String,
    store: File,
    data: File,
    temp: File,
    db: File,
    device: u64,
}

impl<'a> PinnedFsStore<'a> {
    pub fn open(volume: &'a StagingVolumeGuard, store_child: &str) -> Result<Self, FdHold> {
        if !valid_component(store_child) {
            return Err(FdHold::InvalidChildName);
        }
        volume.check()?;
        let mount = volume.pinned_directory();
        let device = mount.metadata()?.dev();
        let store = child(mount, store_child, true)?;
        check_dir(&store, device)?;
        let data = child(&store, "data", true)?;
        check_dir(&data, device)?;
        let temp = child(&store, "temp", true)?;
        check_dir(&temp, device)?;
        let db = child(&store, "blobs.db", false)?;
        check_regular(&db, device, MAX_DB)?;
        let pinned = Self {
            volume,
            store_child: store_child.to_owned(),
            store,
            data,
            temp,
            db,
            device,
        };
        pinned.check_bindings()?;
        Ok(pinned)
    }

    /// Re-resolve every exact known directory/file name from the held mount
    /// descriptor. Rename/replace races are HOLD even if substituted contents
    /// have the same hash. This does NOT stop in-place writes by a privileged
    /// uncooperative process or a swap-back race between checks.
    pub fn check_bindings(&self) -> Result<(), FdHold> {
        self.volume.check()?;
        let new_store = child(self.volume.pinned_directory(), &self.store_child, true)?;
        if !same_node(&new_store.metadata()?, &self.store.metadata()?) {
            return Err(FdHold::ChangedDirectoryBinding);
        }
        for (name, pinned) in [("data", &self.data), ("temp", &self.temp)] {
            let current = child(&self.store, name, true)?;
            if !same_node(&current.metadata()?, &pinned.metadata()?) {
                return Err(FdHold::ChangedDirectoryBinding);
            }
        }
        let current = child(&self.store, "blobs.db", false)?;
        if !same_file(&current.metadata()?, &self.db.metadata()?) {
            return Err(FdHold::ChangedFileBinding);
        }
        self.volume.check()?;
        Ok(())
    }

    fn open_blob_part(&self, hash: [u8; 32], extension: &str, max: u64)
        -> Result<(String, File, Metadata), FdHold>
    {
        let hex = blake3::Hash::from_bytes(hash).to_hex();
        let name = format!("{hex}.{extension}");
        let file = child(&self.data, &name, false)?;
        let metadata = check_regular(&file, self.device, max)?;
        Ok((name, file, metadata))
    }

    fn check_blob_binding(&self, name: &str, file: &File, before: &Metadata)
        -> Result<(), FdHold>
    {
        let current = child(&self.data, name, false)?;
        if !same_file(&current.metadata()?, before)
            || !unchanged_in_place(before, &file.metadata()?)
        {
            return Err(FdHold::ChangedFileBinding);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FdCustodyEvidenceOnly {
    pub verified_object_bytes: u64,
    pub verified_external_bao_bytes: u64,
    pub verified_journal_generation: u64,
    pub fd_relative_no_symlink_no_xdev: bool,
    pub pinned_db_opened_read_only: bool,
    pub pinned_data_and_bao_authenticated: bool,
    pub independent_writer_exclusion_unproven: bool,
    pub partial_bitfields_unchecked: bool,
    pub atomic_filesystem_snapshot: bool,
    pub file_gate_ready: bool,
}

/// P2P-11a: full crypto on two *pinned file descriptors* while an independent
/// redb read-only database handle remains open on exactly the pinned DB inode.
/// The prerequisite joined audit still runs serially by pathname: this new
/// gate is NOT a single atomic transaction with that older audit.
pub fn audit_fd_complete_owned(
    volume: &StagingVolumeGuard,
    ledger: &DurableLedger,
    store_child: &str,
    expected_hash: [u8; 32],
    expected_len: u64,
) -> Result<FdCustodyEvidenceOnly, FdHold> {
    if expected_len <= BLOCK.bytes() as u64 || expected_len > MAX_DATA {
        return Err(FdHold::InvalidObjectLength);
    }
    let start = ledger.snapshot()?;
    let found = ledger.reservation_inventory()?
        .into_iter()
        .find(|r| r.hash == expected_hash)
        .ok_or(FdHold::UnverifiedOrUnknownReservation)?;
    if !found.verified || found.expected_bytes != expected_len {
        return Err(FdHold::UnverifiedOrUnknownReservation);
    }
    let joined = audit_quiescent_staging(volume, ledger, store_child)?;
    if joined.ledger_generation != start.generation
        || joined.verified_external_blobs == 0
        || joined.verified_total_bytes < expected_len
        || joined.file_gate_ready
        || joined.atomic_filesystem_snapshot
    {
        return Err(FdHold::UnverifiedOrUnknownReservation);
    }

    let pinned = PinnedFsStore::open(volume, store_child)?;
    let db_before = pinned.db.metadata()?;
    let db_digest = digest_pinned(&pinned.db, db_before.len(), MAX_DB)?;
    // /proc/self/fd is constructed solely from our own already pinned,
    // CLOEXEC-protected DB FD. No DB path from redb/remote data is followed.
    // On a system lacking procfs this fails closed; never reopen by pathname.
    let proc_db = format!("/proc/self/fd/{}", pinned.db.as_raw_fd());
    let ro_db = ReadOnlyDatabase::open(&proc_db)
        .map_err(|_| FdHold::RedbReadOnlyUnavailable)?;
    let read_tx = ro_db.begin_read().map_err(|_| FdHold::RedbReadOnlyUnavailable)?;

    let (data_name, data, data_before) = pinned.open_blob_part(expected_hash, "data", MAX_DATA)?;
    let (bao_name, bao, bao_before) = pinned.open_blob_part(expected_hash, "obao4", MAX_OUTBOARD)?;
    if data_before.len() != expected_len {
        return Err(FdHold::InvalidObjectLength);
    }
    let tree = BaoTree::new(expected_len, BLOCK);
    if tree.outboard_size() == 0 || bao_before.len() != tree.outboard_size() {
        return Err(FdHold::NonCanonicalOutboard);
    }
    if digest_pinned(&data, expected_len, MAX_DATA)? != expected_hash {
        return Err(FdHold::WrongDataHash);
    }
    encode_ranges_validated(
        &data,
        PreOrderOutboard {
            root: blake3::Hash::from_bytes(expected_hash),
            tree,
            data: &bao,
        },
        &ChunkRanges::all(),
        io::sink(),
    ).map_err(|_| FdHold::InvalidBaoProof)?;

    pinned.check_blob_binding(&data_name, &data, &data_before)?;
    pinned.check_blob_binding(&bao_name, &bao, &bao_before)?;
    pinned.check_bindings()?;
    if !unchanged_in_place(&db_before, &pinned.db.metadata()?)
        || digest_pinned(&pinned.db, db_before.len(), MAX_DB)? != db_digest
    {
        return Err(FdHold::DatabaseChanged);
    }
    if ledger.snapshot()? != start {
        return Err(FdHold::UnverifiedOrUnknownReservation);
    }
    drop(read_tx);
    drop(ro_db);
    pinned.check_bindings()?;

    Ok(FdCustodyEvidenceOnly {
        verified_object_bytes: expected_len,
        verified_external_bao_bytes: bao_before.len(),
        verified_journal_generation: start.generation,
        fd_relative_no_symlink_no_xdev: true,
        pinned_db_opened_read_only: true,
        pinned_data_and_bao_authenticated: true,
        independent_writer_exclusion_unproven: true,
        partial_bitfields_unchecked: true,
        atomic_filesystem_snapshot: false,
        file_gate_ready: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn openat2_refuses_symlink_and_relative_escape_without_fallback() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("real"), b"immutable-test").unwrap();
        symlink("real", dir.path().join("alias")).unwrap();
        let root = File::open(dir.path()).unwrap();
        let real = child(&root, "real", false).unwrap();
        assert_eq!(real.metadata().unwrap().len(), 14);
        assert!(matches!(
            child(&root, "alias", false),
            Err(FdHold::UnsafeRelativePath)
        ));
        for bad in ["..", "../real", "a/b", "/etc/passwd", "", "a..b"] {
            assert!(matches!(child(&root, bad, false), Err(FdHold::InvalidChildName)));
        }
    }
}
