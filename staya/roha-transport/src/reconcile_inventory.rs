//! P2P-BLOB-09a: read-only, fail-closed EXTERNAL FILE inventory
//! of the pinned iroh-blobs 0.103.1 FsStore layout.
//!
//! Important: this does NOT parse or authenticate the redb blobs.db tables.
//! Small blobs can be inline in that database, so positive inventory evidence
//! NEVER implies a complete, trusted or File Gate-ready workspace.
//! No files are written, renamed, deleted, or repaired by this module.
//! Production also requires fd-relative custody and exclusive FsStore owner.

#![cfg(target_os = "linux")]

use crate::{
    bounded_staging::{StagingVolumeGuard, VolumeError},
    durable_admission::{DurableError, DurableLedger, ReservedBlobAudit},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, Metadata},
    io::{self, Read},
    os::unix::fs::MetadataExt,
    path::Path,
};

const MAX_DIR_FILES: usize = 4096;
const MAX_VERIFIED_BYTES_TO_HASH: u64 = 256 * 1024 * 1024;

#[derive(Debug)]
pub enum AuditHold {
    Volume(VolumeError),
    Ledger(DurableError),
    Io(io::Error),
    UnsafeStoreRoot,
    MissingOrUnsafeDatabase,
    UnexpectedStoreEntry(String),
    UnexpectedDataEntry(String),
    OrphanExternalHash(String),
    ResidualTemp(String),
    InvalidFileType(String),
    WrongDevice(String),
    TooManyFiles,
    MissingVerifiedExternalData(String),
    OversizeExternalData(String),
    WrongVerifiedDataSize(String),
    WrongVerifiedDataHash(String),
}
impl From<io::Error> for AuditHold {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}
impl From<VolumeError> for AuditHold {
    fn from(err: VolumeError) -> Self {
        Self::Volume(err)
    }
}
impl From<DurableError> for AuditHold {
    fn from(err: DurableError) -> Self {
        Self::Ledger(err)
    }
}

/// Read-only external-path evidence. Both safety flags stay false/true even
/// when all the observed external payload bytes match their ledger hashes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InventoryEvidenceOnly {
    pub journal_generation: u64,
    pub reserved_objects: usize,
    pub reserved_bytes: u64,
    pub external_files: usize,
    pub verified_external_data: usize,
    pub verified_external_bytes: u64,
    pub database_contents_uninspected: bool,
    pub file_gate_ready: bool,
}

#[derive(Default)]
struct ExternalRoles {
    data_len: Option<u64>,
    _outboard: bool,
    _sizes: bool,
    _bitfield: bool,
}

#[derive(Clone, Copy)]
enum Suffix {
    Data,
    Outboard,
    Sizes,
    Bitfield,
}

fn parse_data_name(name: &str) -> Option<(&str, Suffix)> {
    let (stem, suffix) = name.rsplit_once('.')?;
    if stem.len() != 64
        || !stem
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return None;
    }
    let suffix = match suffix {
        "data" => Suffix::Data,
        "obao4" => Suffix::Outboard,
        "sizes4" => Suffix::Sizes,
        "bitfield" => Suffix::Bitfield,
        _ => return None,
    };
    Some((stem, suffix))
}

fn checked_regular_file(meta: &Metadata, dev: u64, name: &str) -> Result<(), AuditHold> {
    if !meta.file_type().is_file() || meta.nlink() != 1 {
        return Err(AuditHold::InvalidFileType(name.to_owned()));
    }
    if meta.dev() != dev {
        return Err(AuditHold::WrongDevice(name.to_owned()));
    }
    Ok(())
}

fn checked_directory(path: &Path, dev: u64) -> Result<(), AuditHold> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_dir() {
        return Err(AuditHold::InvalidFileType(path.display().to_string()));
    }
    if meta.dev() != dev {
        return Err(AuditHold::WrongDevice(path.display().to_string()));
    }
    Ok(())
}

fn full_hash_matches(path: &Path, size: u64, hash: &[u8; 32]) -> Result<bool, AuditHold> {
    let mut file = File::open(path)?;
    if file.metadata()?.len() != size {
        return Ok(false);
    }
    let mut digest = blake3::Hasher::new();
    let mut total = 0u64;
    let mut block = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut block)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or(AuditHold::TooManyFiles)?;
        if total > size {
            return Ok(false);
        }
        digest.update(&block[..read]);
    }
    Ok(total == size && digest.finalize().as_bytes() == hash)
}

/// Inventory an already-shut-down pinned FsStore directory directly under an
/// independently bounded mount. A live exclusive DurableLedger owner must be
/// held throughout the scan; FsStore must have a separately exclusive owner.
/// This is only evidence gathering, not authorization, and has pathname TOCTOU.
pub fn audit_external_inventory(
    volume: &StagingVolumeGuard,
    ledger: &DurableLedger,
    store_child: &str,
) -> Result<InventoryEvidenceOnly, AuditHold> {
    audit_external_inventory_with_inline(volume, ledger, store_child, &BTreeSet::new())
}

/// Strictly internal joint-audit path: only hashes already verified by the
/// exact pinned read-only redb scan may have no external .data file.
/// This is NOT public caller-supplied authority, nor a File Gate clearance.
pub(crate) fn audit_external_inventory_with_inline(
    volume: &StagingVolumeGuard,
    ledger: &DurableLedger,
    store_child: &str,
    verified_db_inline: &BTreeSet<[u8; 32]>,
) -> Result<InventoryEvidenceOnly, AuditHold> {
    if store_child.is_empty()
        || store_child == "."
        || store_child == ".."
        || store_child.contains('/')
        || store_child.contains('\\')
    {
        return Err(AuditHold::UnsafeStoreRoot);
    }
    volume.check()?;
    let root = volume.root().join(store_child);
    let dev = fs::symlink_metadata(volume.root())?.dev();
    checked_directory(&root, dev)?;

    let records = ledger.reservation_inventory()?;
    let snapshot = ledger.snapshot()?;
    if records.len() != snapshot.objects || records.len() > MAX_DIR_FILES {
        return Err(AuditHold::TooManyFiles);
    }
    let mut expected = BTreeMap::<String, ReservedBlobAudit>::new();
    for item in records {
        let hex = blake3::Hash::from_bytes(item.hash).to_hex().to_string();
        if expected.insert(hex, item).is_some() {
            return Err(AuditHold::TooManyFiles);
        }
    }

    for hash in verified_db_inline {
        let name = blake3::Hash::from_bytes(*hash).to_hex().to_string();
        match expected.get(&name) {
            Some(record) if record.verified => {}
            _ => return Err(AuditHold::OrphanExternalHash(name)),
        }
    }

    let mut seen_db = false;
    let mut seen_data = false;
    let mut seen_temp = false;
    let mut count = 0usize;
    for item in fs::read_dir(&root)? {
        count += 1;
        let item = item?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| AuditHold::UnexpectedStoreEntry("non-UTF8".to_owned()))?;
        let meta = fs::symlink_metadata(item.path())?;
        match name.as_str() {
            "blobs.db" => {
                checked_regular_file(&meta, dev, &name)?;
                if meta.len() == 0 {
                    return Err(AuditHold::MissingOrUnsafeDatabase);
                }
                seen_db = true;
            }
            "data" => {
                checked_directory(&item.path(), dev)?;
                seen_data = true;
            }
            "temp" => {
                checked_directory(&item.path(), dev)?;
                seen_temp = true;
            }
            _ => return Err(AuditHold::UnexpectedStoreEntry(name)),
        }
        // Check the exact layout first. read_dir enumeration order is NOT
        // stable: an unknown fourth filename must be UnexpectedStoreEntry
        // whether encountered first or last, never a misleading size error.
        if count > 3 {
            return Err(AuditHold::TooManyFiles);
        }
    }
    if !seen_db {
        return Err(AuditHold::MissingOrUnsafeDatabase);
    }
    if !seen_data || !seen_temp {
        return Err(AuditHold::UnsafeStoreRoot);
    }

    // After clean shutdown ANY leftover temporary file is a HOLD, regardless
    // of whether its prefix appears to belong to a known journal object.
    for (index, entry) in fs::read_dir(root.join("temp"))?.enumerate() {
        if index >= MAX_DIR_FILES {
            return Err(AuditHold::TooManyFiles);
        }
        let entry = entry?;
        return Err(AuditHold::ResidualTemp(
            entry.file_name().to_string_lossy().to_string(),
        ));
    }

    let mut roles = BTreeMap::<String, ExternalRoles>::new();
    let mut external_files = 0usize;
    for item in fs::read_dir(root.join("data"))? {
        external_files += 1;
        if external_files > MAX_DIR_FILES {
            return Err(AuditHold::TooManyFiles);
        }
        let item = item?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| AuditHold::UnexpectedDataEntry("non-UTF8".to_owned()))?;
        let (stem, kind) =
            parse_data_name(&name).ok_or_else(|| AuditHold::UnexpectedDataEntry(name.clone()))?;
        let expectation = expected
            .get(stem)
            .ok_or_else(|| AuditHold::OrphanExternalHash(name.clone()))?;
        let meta = fs::symlink_metadata(item.path())?;
        checked_regular_file(&meta, dev, &name)?;
        let seen = roles.entry(stem.to_owned()).or_default();
        match kind {
            Suffix::Data => {
                if meta.len() > expectation.expected_bytes {
                    return Err(AuditHold::OversizeExternalData(name));
                }
                seen.data_len = Some(meta.len());
            }
            Suffix::Outboard => seen._outboard = true,
            Suffix::Sizes => seen._sizes = true,
            Suffix::Bitfield => seen._bitfield = true,
        }
    }

    let mut bytes_to_hash = 0u64;
    let mut verified_external = 0usize;
    let mut verified_bytes = 0u64;
    for (name, entry) in &expected {
        if !entry.verified {
            continue;
        }
        if verified_db_inline.contains(&entry.hash) {
            // A complete inline blob was already length/BLAKE3 verified in
            // pinned redb. Reject any stray external file under its hash
            // instead of treating an extra copy as harmless disk usage.
            if roles.contains_key(name) {
                return Err(AuditHold::UnexpectedDataEntry(name.clone()));
            }
            continue;
        }
        // A verified object may be validly inlined into redb. We cannot
        // prove that without a trusted DB parser, so leave it in HOLD.
        let len = roles
            .get(name)
            .and_then(|r| r.data_len)
            .ok_or_else(|| AuditHold::MissingVerifiedExternalData(name.clone()))?;
        if len != entry.expected_bytes {
            return Err(AuditHold::WrongVerifiedDataSize(name.clone()));
        }
        bytes_to_hash = bytes_to_hash
            .checked_add(len)
            .ok_or(AuditHold::TooManyFiles)?;
        if bytes_to_hash > MAX_VERIFIED_BYTES_TO_HASH {
            return Err(AuditHold::TooManyFiles);
        }
        if !full_hash_matches(
            &root.join("data").join(format!("{name}.data")),
            len,
            &entry.hash,
        )? {
            return Err(AuditHold::WrongVerifiedDataHash(name.clone()));
        }
        verified_external += 1;
        verified_bytes += len;
    }
    volume.check()?;
    Ok(InventoryEvidenceOnly {
        journal_generation: snapshot.generation,
        reserved_objects: snapshot.objects,
        reserved_bytes: snapshot.reserved_bytes,
        external_files,
        verified_external_data: verified_external,
        verified_external_bytes: verified_bytes,
        database_contents_uninspected: true,
        file_gate_ready: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pinned_lowercase_hex_and_file_suffixes_are_accepted() {
        let hash = "a".repeat(64);
        for suffix in ["data", "obao4", "sizes4", "bitfield"] {
            assert!(parse_data_name(&format!("{hash}.{suffix}")).is_some());
        }
        for bad in [
            format!("{hash}.temp"),
            format!("{}.data", "A".repeat(64)),
            format!("{}.data", "a".repeat(63)),
            format!("{hash}.data.bak"),
            format!("{}.data", "z".repeat(64)),
        ] {
            assert!(parse_data_name(&bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn matches_full_content_not_only_file_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lab-only-payload");
        let valid = b"trusted pinned payload";
        let fake = b"altered pinned payload";
        assert_eq!(valid.len(), fake.len());
        fs::write(&path, valid).unwrap();
        let hash = *blake3::hash(valid).as_bytes();
        assert!(full_hash_matches(&path, valid.len() as u64, &hash).unwrap());
        fs::write(&path, fake).unwrap();
        assert!(!full_hash_matches(&path, valid.len() as u64, &hash).unwrap());
        assert!(!full_hash_matches(&path, (valid.len() + 1) as u64, &hash).unwrap());
    }
}
