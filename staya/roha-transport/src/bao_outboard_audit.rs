//! P2P-BLOB-10b: cryptographic verification of a *complete* pinned iroh-blobs
//! 0.103.1 PRE-ORDER on-disk Bao outboard against a journal-approved BLAKE3 root.
//!
//! The original FsStore MUST already be shut down. This module never opens a
//! writable FsStore, modifies redb, accepts remote outboard claims, publishes
//! user files, or frees quota. Full-file checks are deliberately scoped to the
//! OWNED *.data + *.obao4 case (> 16KiB inline-outboard threshold).
//!
//! Range validity comes from bao-tree 0.16.1::io::sync::encode_ranges_validated,
//! the same pinned Bao tree used upstream with IROH_BLOCK_SIZE = 16KiB.
//! The encoder reads all authenticated chunks but writes only to std::io::sink.
//!
//! It is NOT a comprehensive P2P staging permit: inlined Bao, incomplete
//! bitfields, race-free fd-relative custody, malicious concurrent mounts,
//! on-disk power loss and cross-platform storage are not solved here.

#![cfg(target_os = "linux")]

use std::{
    fs::{self, File, Metadata},
    io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use bao_tree::{
    BaoTree, BlockSize, ChunkRanges,
    io::{outboard::PreOrderOutboard, sync::encode_ranges_validated},
};

use crate::{
    bounded_staging::{StagingVolumeGuard, VolumeError},
    durable_admission::{DurableError, DurableLedger},
    joint_staging_audit::{JointHold, audit_quiescent_staging},
};

const IROH_BLOCK_SIZE: BlockSize = BlockSize::from_chunk_log(4);
const MAX_COMPLETE_OBJECT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_OUTBOARD_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug)]
pub enum BaoHold {
    Volume(VolumeError),
    Ledger(DurableError),
    Joint(JointHold),
    Io(io::Error),
    BadStoreName,
    MissingAuthorizedReservation,
    UnverifiedReservation,
    SizePolicyViolation,
    MissingOrUnsafeDataFile,
    MissingOrUnsafeOutboardFile,
    InvalidLength,
    MissingBaoNodes,
    InvalidBaoProof,
    ChangedFileIdentity,
}

impl From<VolumeError> for BaoHold {
    fn from(value: VolumeError) -> Self {
        Self::Volume(value)
    }
}
impl From<DurableError> for BaoHold {
    fn from(value: DurableError) -> Self {
        Self::Ledger(value)
    }
}
impl From<JointHold> for BaoHold {
    fn from(value: JointHold) -> Self {
        Self::Joint(value)
    }
}
impl From<io::Error> for BaoHold {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OwnedBaoProofEvidenceOnly {
    pub object_bytes: u64,
    pub outboard_bytes: u64,
    pub verified_journal_generation: u64,
    pub complete_preorder_proof_valid: bool,
    pub source_files_unchanged: bool,
    pub file_gate_ready: bool,
    pub inlined_bao_unchecked: bool,
    pub partial_bitfields_unchecked: bool,
    pub atomic_filesystem_snapshot: bool,
}

fn checked_file(
    path: &Path,
    expected_device: u64,
    upper_bound: u64,
    is_outboard: bool,
) -> Result<(File, Metadata), BaoHold> {
    let meta = fs::symlink_metadata(path).map_err(|_| {
        if is_outboard {
            BaoHold::MissingOrUnsafeOutboardFile
        } else {
            BaoHold::MissingOrUnsafeDataFile
        }
    })?;
    if !meta.file_type().is_file()
        || meta.nlink() != 1
        || meta.dev() != expected_device
        || meta.len() > upper_bound
    {
        return Err(if is_outboard {
            BaoHold::MissingOrUnsafeOutboardFile
        } else {
            BaoHold::MissingOrUnsafeDataFile
        });
    }
    let file = File::open(path)?;
    let opened = file.metadata()?;
    if opened.dev() != meta.dev() || opened.ino() != meta.ino() || opened.len() != meta.len() {
        return Err(BaoHold::ChangedFileIdentity);
    }
    Ok((file, opened))
}

fn fully_verify_preorder(
    data: &File,
    outboard: &File,
    size: u64,
    authorized_hash: [u8; 32],
) -> Result<(), BaoHold> {
    let proof = PreOrderOutboard {
        root: blake3::Hash::from_bytes(authorized_hash),
        tree: BaoTree::new(size, IROH_BLOCK_SIZE),
        data: outboard,
    };
    // Every byte/chunk and every interior parent needed for a complete
    // outboard is checked against its trusted root. Never stream the raw
    // content to an untrusted process; only discard encoded output.
    encode_ranges_validated(data, proof, &ChunkRanges::all(), io::sink())
        .map_err(|_| BaoHold::InvalidBaoProof)
}

/// Inspect an ACTUAL pre-order *.obao4 backed by one isolated bounded mount.
/// The journal is already exclusively owned and metadata is cross-checked with
/// redb and external data through the read-only P2P-10a audit.
/// WARNING: this still does NOT provide an atomic VFS snapshot under races.
pub fn audit_complete_owned_preorder(
    mount: &StagingVolumeGuard,
    ledger: &DurableLedger,
    store_child: &str,
    trusted_hash: [u8; 32],
    known_bytes: u64,
) -> Result<OwnedBaoProofEvidenceOnly, BaoHold> {
    if store_child.is_empty()
        || store_child == "."
        || store_child == ".."
        || store_child.contains('/')
        || store_child.contains('\\')
    {
        return Err(BaoHold::BadStoreName);
    }
    if known_bytes <= IROH_BLOCK_SIZE.bytes() as u64 || known_bytes > MAX_COMPLETE_OBJECT_BYTES {
        return Err(BaoHold::SizePolicyViolation);
    }
    mount.check()?;
    let found = ledger
        .reservation_inventory()?
        .into_iter()
        .find(|x| x.hash == trusted_hash)
        .ok_or(BaoHold::MissingAuthorizedReservation)?;
    if !found.verified {
        return Err(BaoHold::UnverifiedReservation);
    }
    if found.expected_bytes != known_bytes {
        return Err(BaoHold::InvalidLength);
    }
    // Confirm complete data + exact DB layout and this one locked journal
    // before testing Bao's own internal parent/subtree hashes.
    let joined = audit_quiescent_staging(mount, ledger, store_child)?;
    if joined.verified_external_blobs == 0
        || joined.verified_total_bytes < known_bytes
        || joined.ledger_generation != ledger.snapshot()?.generation
    {
        return Err(BaoHold::MissingAuthorizedReservation);
    }

    let root = mount.root().join(store_child).join("data");
    let device = fs::symlink_metadata(mount.root())?.dev();
    let hex = blake3::Hash::from_bytes(trusted_hash).to_hex().to_string();
    let data_path: PathBuf = root.join(format!("{hex}.data"));
    let ob_path: PathBuf = root.join(format!("{hex}.obao4"));
    let (data, data_meta) = checked_file(&data_path, device, MAX_COMPLETE_OBJECT_BYTES, false)?;
    let (outboard, ob_meta) = checked_file(&ob_path, device, MAX_OUTBOARD_BYTES, true)?;
    if data_meta.len() != known_bytes || ob_meta.len() == 0 {
        return Err(BaoHold::InvalidLength);
    }

    fully_verify_preorder(&data, &outboard, known_bytes, trusted_hash)?;
    // Detect obvious in-place changes by a cooperating writer. Privileged
    // replacements/remount races still require a future fd-relative adapter.
    let end_data = data.metadata()?;
    let end_ob = outboard.metadata()?;
    if data_meta.len() != end_data.len()
        || data_meta.ino() != end_data.ino()
        || data_meta.dev() != end_data.dev()
        || ob_meta.len() != end_ob.len()
        || ob_meta.ino() != end_ob.ino()
        || ob_meta.dev() != end_ob.dev()
    {
        return Err(BaoHold::ChangedFileIdentity);
    }
    mount.check()?;
    Ok(OwnedBaoProofEvidenceOnly {
        object_bytes: known_bytes,
        outboard_bytes: ob_meta.len(),
        verified_journal_generation: joined.ledger_generation,
        complete_preorder_proof_valid: true,
        source_files_unchanged: true,
        file_gate_ready: false,
        inlined_bao_unchecked: true,
        partial_bitfields_unchecked: true,
        atomic_filesystem_snapshot: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bao_tree::io::sync::CreateOutboard;

    #[test]
    fn preorder_requires_matching_tree_and_rejects_tampered_proof() {
        let dir = tempfile::tempdir().unwrap();
        let original = (0..65553usize)
            .map(|x| (x % 253) as u8)
            .collect::<Vec<u8>>();
        let root_hash = *blake3::hash(&original).as_bytes();
        let data_path = dir.path().join("data");
        let ob_path = dir.path().join("outboard");
        fs::write(&data_path, &original).unwrap();
        let generated =
            PreOrderOutboard::<Vec<u8>>::create(File::open(&data_path).unwrap(), IROH_BLOCK_SIZE)
                .unwrap();
        assert_eq!(generated.root.as_bytes(), &root_hash);
        fs::write(&ob_path, generated.data).unwrap();
        fully_verify_preorder(
            &File::open(&data_path).unwrap(),
            &File::open(&ob_path).unwrap(),
            original.len() as u64,
            root_hash,
        )
        .unwrap();
        let mut wrong = fs::read(&ob_path).unwrap();
        assert!(
            !wrong.is_empty(),
            "size > one tree group must have proof parents"
        );
        wrong[0] ^= 0x40;
        fs::write(&ob_path, wrong).unwrap();
        assert!(matches!(
            fully_verify_preorder(
                &File::open(&data_path).unwrap(),
                &File::open(&ob_path).unwrap(),
                original.len() as u64,
                root_hash,
            ),
            Err(BaoHold::InvalidBaoProof)
        ));
    }
}
