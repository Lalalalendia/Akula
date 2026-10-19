//! P2P-BLOB-09b: pinned redb 4.4.0 / iroh-blobs 0.103.1 metadata audit.
//!
//! Strictly read-only: uses redb::ReadOnlyDatabase, NEVER Database::open,
//! create, begin_write, repair or FsStore::load. No implicit metadata repair.
//! This is not product sync authority, not a File Gate permit, and not a
//! replacement for P2P-09a's external-file inventory or Bao verification.
//! The caller must hold the P2P-07 owner lock, have a quiet/disconnected
//! FsStore, and independently guard the physical Linux staging mount.
//!
//! Raw redb Value adapters mirror *only the exact type names* of the pinned
//! version. Unlike upstream EntryState::from_bytes (which unwraps postcard),
//! its bounded wire inspection rejects malformed/unrecognized data without
//! panics and never interprets an external user filepath as trusted.

#![cfg(target_os = "linux")]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, Read},
    os::unix::fs::MetadataExt,
};

use redb::{
    Key, ReadOnlyDatabase, ReadableDatabase, ReadableTable, TableDefinition,
    TableHandle, Value,
};

use crate::{
    bounded_staging::{StagingVolumeGuard, VolumeError},
    durable_admission::{DurableError, DurableLedger, ReservedBlobAudit},
};

const MAX_DB_BYTES: u64 = 64 * 1024 * 1024;
const MAX_BLOBS: usize = 256;
const MAX_TAGS: usize = 256;
const MAX_INLINE_BYTES: usize = 16 * 1024 * 1024;
const MAX_ENTRY_WIRE_BYTES: usize = 256;

#[derive(Debug)]
pub enum RedbHold {
    Volume(VolumeError),
    Ledger(DurableError),
    Io(io::Error),
    InvalidStoreRoot,
    DatabaseUnsafe,
    UnsupportedSchema,
    UnknownOrDamagedRecord,
    OrphanDatabaseBlob,
    MissingDatabaseBlob,
    JournalNotVerified,
    WrongDeclaredLength,
    InvalidInlineData,
    MissingInlineData,
    OrphanInlineData,
    InvalidInlineOutboard,
    MissingInlineOutboard,
    OrphanInlineOutboard,
    ForeignOrUnsupportedTag,
    TooManyRecords,
    DatabaseChangedDuringRead,
}

impl From<VolumeError> for RedbHold {
    fn from(value: VolumeError) -> Self { Self::Volume(value) }
}
impl From<DurableError> for RedbHold {
    fn from(value: DurableError) -> Self { Self::Ledger(value) }
}
impl From<io::Error> for RedbHold {
    fn from(value: io::Error) -> Self { Self::Io(value) }
}

// Pinned source: iroh-blobs tag v0.103.1:
// src/hash.rs -> Hash type_name "iroh_blobs::Hash", width 32
// src/hash.rs -> HashAndFormat "iroh_blobs::HashAndFormat", width 33
// src/store/util.rs -> Tag "Tag"
// src/store/fs/entry_state.rs -> EntryState "EntryState" (postcard).
// A &[u8] Value (not a custom type) is used for the inline tables.
macro_rules! raw_value {
    ($name:ident, $type_name:literal, $width:expr) => {
        #[derive(Debug)]
        struct $name;
        impl Value for $name {
            type SelfType<'a> = &'a [u8];
            type AsBytes<'a> = &'a [u8];
            fn fixed_width() -> Option<usize> { $width }
            fn from_bytes<'a>(value: &'a [u8]) -> Self::SelfType<'a>
            where Self: 'a { value }
            fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
            where Self: 'a, Self: 'b { *value }
            fn type_name() -> redb::TypeName { redb::TypeName::new($type_name) }
        }
    };
}
raw_value!(DbHash, "iroh_blobs::Hash", Some(32));
raw_value!(DbState, "EntryState", None);
raw_value!(DbTag, "Tag", None);
raw_value!(DbHashFormat, "iroh_blobs::HashAndFormat", Some(33));
impl Key for DbHash {
    fn compare(left: &[u8], right: &[u8]) -> std::cmp::Ordering { left.cmp(right) }
}
impl Key for DbTag {
    fn compare(left: &[u8], right: &[u8]) -> std::cmp::Ordering { left.cmp(right) }
}

const BLOBS: TableDefinition<DbHash, DbState> = TableDefinition::new("blobs-0");
const TAGS: TableDefinition<DbTag, DbHashFormat> = TableDefinition::new("tags-0");
const INLINE: TableDefinition<DbHash, &[u8]> = TableDefinition::new("inline-data-0");
const OUTBOARD: TableDefinition<DbHash, &[u8]> = TableDefinition::new("inline-outboard-0");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataLocation {
    Inline,
    Owned(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutboardLocation {
    Inline,
    Owned,
    NotNeeded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CompleteBlob {
    data: DataLocation,
    outboard: OutboardLocation,
}

/// postcard 1.x encodes enum variant indices, u64 and Option discriminants
/// as unsigned LEB128. Bound the input and reject noncanonical or oversized
/// values BEFORE calling any upstream panic-on-decode implementation.
fn unsigned_varint(raw: &[u8], offset: &mut usize) -> Result<u64, RedbHold> {
    let mut value = 0u64;
    for i in 0..10usize {
        let octet = *raw.get(*offset).ok_or(RedbHold::UnknownOrDamagedRecord)?;
        *offset += 1;
        if i == 9 && octet > 1 {
            return Err(RedbHold::UnknownOrDamagedRecord);
        }
        value |= u64::from(octet & 0x7f) << (7 * i);
        if octet & 0x80 == 0 {
            if i > 0 && octet == 0 {
                return Err(RedbHold::UnknownOrDamagedRecord);
            }
            return Ok(value);
        }
    }
    Err(RedbHold::UnknownOrDamagedRecord)
}

/// Parse only COMPLETE, store-owned or inline entries. Partial and External
/// path entries are deliberately HOLD until separate custody proof exists.
fn parse_complete(raw: &[u8]) -> Result<CompleteBlob, RedbHold> {
    if raw.len() > MAX_ENTRY_WIRE_BYTES {
        return Err(RedbHold::UnknownOrDamagedRecord);
    }
    let mut at = 0usize;
    if unsigned_varint(raw, &mut at)? != 0 {
        return Err(RedbHold::JournalNotVerified);
    }
    let data = match unsigned_varint(raw, &mut at)? {
        0 => DataLocation::Inline,
        1 => DataLocation::Owned(unsigned_varint(raw, &mut at)?),
        // External(paths, len): never follow paths supplied by a foreign DB.
        _ => return Err(RedbHold::UnknownOrDamagedRecord),
    };
    let outboard = match unsigned_varint(raw, &mut at)? {
        0 => OutboardLocation::Inline,
        1 => OutboardLocation::Owned,
        2 => OutboardLocation::NotNeeded,
        _ => return Err(RedbHold::UnknownOrDamagedRecord),
    };
    if at != raw.len() {
        return Err(RedbHold::UnknownOrDamagedRecord);
    }
    Ok(CompleteBlob { data, outboard })
}

fn hash_key(raw: &[u8]) -> Result<[u8; 32], RedbHold> {
    raw.try_into().map_err(|_| RedbHold::UnknownOrDamagedRecord)
}

fn sha_of_file(path: &std::path::Path) -> Result<[u8; 32], RedbHold> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 { break; }
        hasher.update(&buffer[..size]);
    }
    Ok(*hasher.finalize().as_bytes())
}

/// Read-only metadata evidence. Outboard cryptographic correctness, disk
/// filesystem inventory and the signed peer authorization remain separate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RedbEvidenceOnly {
    pub journal_generation: u64,
    pub blob_records: usize,
    pub inline_data_entries: usize,
    pub inline_outboards: usize,
    pub tag_references: usize,
    pub verified_inline_bytes: u64,
    pub database_unchanged: bool,
    pub file_gate_ready: bool,
    pub bao_verification_pending: bool,
}

/// Every table and blob record in the pinned DB must match an already-held
/// durable journal reservation. The read-only redb handle takes a shared lock
/// and never invokes write transactions or auto-repair. A writer must NOT be
/// permitted to create/modify entries concurrently on the staging volume.
pub fn audit_pinned_redb(
    volume: &StagingVolumeGuard,
    ledger: &DurableLedger,
    store_child: &str,
) -> Result<RedbEvidenceOnly, RedbHold> {
    if store_child.is_empty() || store_child == "." || store_child == ".."
        || store_child.contains('/') || store_child.contains('\\')
    {
        return Err(RedbHold::InvalidStoreRoot);
    }
    volume.check()?;
    let root = volume.root().join(store_child);
    let root_meta = fs::symlink_metadata(&root)?;
    let mount_dev = fs::symlink_metadata(volume.root())?.dev();
    if !root_meta.file_type().is_dir() || root_meta.dev() != mount_dev {
        return Err(RedbHold::InvalidStoreRoot);
    }
    let path = root.join("blobs.db");
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.file_type().is_file() || metadata.nlink() != 1
        || metadata.dev() != mount_dev
        || metadata.len() == 0 || metadata.len() > MAX_DB_BYTES
    {
        return Err(RedbHold::DatabaseUnsafe);
    }
    // A byte-for-byte check under the read-only DB lock detects accidental
    // concurrent writes in the lab; it is not race-free fd-relative custody.
    let starting_digest = sha_of_file(&path)?;

    let inventory = ledger.reservation_inventory()?;
    let snapshot = ledger.snapshot()?;
    if inventory.len() > MAX_BLOBS || inventory.len() != snapshot.objects {
        return Err(RedbHold::TooManyRecords);
    }
    let reservations: BTreeMap<[u8; 32], ReservedBlobAudit> =
        inventory.into_iter().map(|x| (x.hash, x)).collect();

    let db = ReadOnlyDatabase::open(&path).map_err(|_| RedbHold::DatabaseUnsafe)?;
    let tx = db.begin_read().map_err(|_| RedbHold::DatabaseUnsafe)?;
    let names: BTreeSet<String> = tx.list_tables()
        .map_err(|_| RedbHold::UnsupportedSchema)?
        .map(|t| t.name().to_owned())
        .collect();
    let pinned: BTreeSet<String> = [
        "blobs-0", "tags-0", "inline-data-0", "inline-outboard-0"
    ].into_iter().map(str::to_owned).collect();
    if names != pinned { return Err(RedbHold::UnsupportedSchema); }

    let blobs = tx.open_table(BLOBS).map_err(|_| RedbHold::UnsupportedSchema)?;
    let mut expected_inline = BTreeSet::new();
    let mut expected_outboard = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for result in blobs.iter().map_err(|_| RedbHold::UnknownOrDamagedRecord)? {
        let (key, raw) = result.map_err(|_| RedbHold::UnknownOrDamagedRecord)?;
        let hash = hash_key(key.value())?;
        let reservation = reservations.get(&hash).ok_or(RedbHold::OrphanDatabaseBlob)?;
        if !seen.insert(hash) || seen.len() > MAX_BLOBS {
            return Err(RedbHold::TooManyRecords);
        }
        // A DB complete blob must have a durable verified reservation. An
        // unverified/partial entry is never cleared by an innocent audit.
        if !reservation.verified {
            return Err(RedbHold::JournalNotVerified);
        }
        let state = parse_complete(raw.value())?;
        match state.data {
            DataLocation::Inline => { expected_inline.insert(hash); }
            DataLocation::Owned(len) if len == reservation.expected_bytes => {}
            DataLocation::Owned(_) => return Err(RedbHold::WrongDeclaredLength),
        }
        if state.outboard == OutboardLocation::Inline {
            expected_outboard.insert(hash);
        }
    }
    for hash in reservations.keys() {
        if !seen.contains(hash) {
            return Err(RedbHold::MissingDatabaseBlob);
        }
    }
    drop(blobs);

    let inline = tx.open_table(INLINE).map_err(|_| RedbHold::UnsupportedSchema)?;
    let mut counted_inline = 0usize;
    let mut inline_bytes = 0usize;
    for row in inline.iter().map_err(|_| RedbHold::InvalidInlineData)? {
        let (key, value) = row.map_err(|_| RedbHold::InvalidInlineData)?;
        let hash = hash_key(key.value())?;
        if !expected_inline.remove(&hash) {
            return Err(RedbHold::OrphanInlineData);
        }
        let body = value.value();
        let expected = &reservations[&hash];
        inline_bytes = inline_bytes.checked_add(body.len())
            .ok_or(RedbHold::TooManyRecords)?;
        if body.len() as u64 != expected.expected_bytes
            || inline_bytes > MAX_INLINE_BYTES
            || blake3::hash(body).as_bytes() != &hash
        {
            return Err(RedbHold::InvalidInlineData);
        }
        counted_inline += 1;
    }
    if !expected_inline.is_empty() {
        return Err(RedbHold::MissingInlineData);
    }
    drop(inline);

    let outboard = tx.open_table(OUTBOARD).map_err(|_| RedbHold::UnsupportedSchema)?;
    let mut outboard_bytes = 0usize;
    let mut outboard_count = 0usize;
    for row in outboard.iter().map_err(|_| RedbHold::InvalidInlineOutboard)? {
        let (key, value) = row.map_err(|_| RedbHold::InvalidInlineOutboard)?;
        let hash = hash_key(key.value())?;
        if !expected_outboard.remove(&hash) {
            return Err(RedbHold::OrphanInlineOutboard);
        }
        outboard_bytes = outboard_bytes.checked_add(value.value().len())
            .ok_or(RedbHold::TooManyRecords)?;
        if outboard_bytes > MAX_INLINE_BYTES {
            return Err(RedbHold::InvalidInlineOutboard);
        }
        outboard_count += 1;
    }
    if !expected_outboard.is_empty() {
        return Err(RedbHold::MissingInlineOutboard);
    }
    drop(outboard);

    let tags = tx.open_table(TAGS).map_err(|_| RedbHold::UnsupportedSchema)?;
    let mut tags_count = 0usize;
    for row in tags.iter().map_err(|_| RedbHold::ForeignOrUnsupportedTag)? {
        let (key, value) = row.map_err(|_| RedbHold::ForeignOrUnsupportedTag)?;
        tags_count += 1;
        if tags_count > MAX_TAGS || key.value().len() > 256 {
            return Err(RedbHold::TooManyRecords);
        }
        let tagged = value.value();
        if tagged.len() != 33 || tagged[32] != 0 {
            // Tagged hash sequences need their own authenticated DAG parser.
            return Err(RedbHold::ForeignOrUnsupportedTag);
        }
        let target = hash_key(&tagged[..32])?;
        if !reservations.contains_key(&target) {
            return Err(RedbHold::ForeignOrUnsupportedTag);
        }
    }
    drop(tags);
    drop(tx);
    drop(db);

    volume.check()?;
    if sha_of_file(&path)? != starting_digest {
        return Err(RedbHold::DatabaseChangedDuringRead);
    }
    Ok(RedbEvidenceOnly {
        journal_generation: snapshot.generation,
        blob_records: seen.len(),
        inline_data_entries: counted_inline,
        inline_outboards: outboard_count,
        tag_references: tags_count,
        verified_inline_bytes: inline_bytes as u64,
        database_unchanged: true,
        file_gate_ready: false,
        bao_verification_pending: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_postcard_complete_owned_wire_and_inline_wire() {
        // Complete=0, DataLocation::Owned=1, size=300 varint,
        // OutboardLocation::Owned=1 (exact pinned upstream variant order).
        let owned = parse_complete(&[0, 1, 0xac, 0x02, 1]).unwrap();
        assert_eq!(owned.data, DataLocation::Owned(300));
        assert_eq!(owned.outboard, OutboardLocation::Owned);
        let inline = parse_complete(&[0, 0, 2]).unwrap();
        assert_eq!(inline.data, DataLocation::Inline);
        assert_eq!(inline.outboard, OutboardLocation::NotNeeded);
    }

    #[test]
    fn reject_partial_external_and_noncanonical_entry_wires() {
        for wire in [
            vec![], vec![1, 0], vec![0, 2], vec![0, 1, 8],
            vec![0, 0, 0, 0], vec![0, 0, 3],
            vec![0, 1, 0x81, 0, 1],
            vec![0, 1, 4, 1, 44],
        ] {
            assert!(parse_complete(&wire).is_err(), "accepted malformed wire {wire:?}");
        }
    }

    #[test]
    fn adapters_match_upstream_redb_schema_names_and_widths() {
        assert_eq!(DbHash::type_name().name(), "iroh_blobs::Hash");
        assert_eq!(DbState::type_name().name(), "EntryState");
        assert_eq!(DbTag::type_name().name(), "Tag");
        assert_eq!(DbHashFormat::type_name().name(), "iroh_blobs::HashAndFormat");
        assert_eq!(DbHash::fixed_width(), Some(32));
        assert_eq!(DbState::fixed_width(), None);
        assert_eq!(DbHashFormat::fixed_width(), Some(33));
    }
}
