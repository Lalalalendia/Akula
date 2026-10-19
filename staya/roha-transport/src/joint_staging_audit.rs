//! P2P-BLOB-10a: fail-closed JOIN of three independently verified views:
//! durable reservation ledger, pinned read-only redb, external FsStore files.
//!
//! Only a *quiescent Linux laboratory* is supported. All three checks run
//! while borrowing one exclusive DurableLedger owner and one mount guard;
//! complete inline blobs are attested by the same redb pass which identifies
//! their exact hashes. This detects stale/missing/foreign cross-component data
//! but IS NOT an atomic filesystem snapshot or a File Gate authorization.
//!
//! A cooperating FsStore must be shut down. Privileged path swaps, untrusted
//! concurrent writes, redb recovery, Bao proof integrity, forced power loss,
//! Windows/Android mounts and signed VaultId-to-Device grants are NOT solved.
//! No file or database writes, deletions, quota releases or repairs occur.

#![cfg(target_os = "linux")]

use crate::{
    bounded_staging::{StagingVolumeGuard, VolumeError},
    durable_admission::{DurableError, DurableLedger},
    reconcile_inventory::{AuditHold, audit_external_inventory_with_inline},
    redb_inventory::{RedbHold, audit_pinned_redb_manifest},
};
use std::{
    fs::File,
    io::{self, Read},
};

const MAX_DATABASE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug)]
pub enum JointHold {
    Volume(VolumeError),
    Ledger(DurableError),
    External(AuditHold),
    Redb(RedbHold),
    Io(io::Error),
    CrossComponentMismatch,
    DatabaseChangedBetweenPhases,
    JournalChangedBetweenPhases,
    DatabaseTooLarge,
}

impl From<VolumeError> for JointHold {
    fn from(x: VolumeError) -> Self {
        Self::Volume(x)
    }
}
impl From<DurableError> for JointHold {
    fn from(x: DurableError) -> Self {
        Self::Ledger(x)
    }
}
impl From<AuditHold> for JointHold {
    fn from(x: AuditHold) -> Self {
        Self::External(x)
    }
}
impl From<RedbHold> for JointHold {
    fn from(x: RedbHold) -> Self {
        Self::Redb(x)
    }
}
impl From<io::Error> for JointHold {
    fn from(x: io::Error) -> Self {
        Self::Io(x)
    }
}

/// Combined *observational* evidence. It is intentionally impossible to
/// construct a File Gate permit from this type. Only the lab holds the
/// quiescent-store precondition; production needs fd-relative isolation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JointEvidenceOnly {
    pub ledger_generation: u64,
    pub durable_blob_count: usize,
    pub verified_external_blobs: usize,
    pub verified_inline_blobs: usize,
    pub verified_total_bytes: u64,
    pub redb_tables_scanned: bool,
    pub database_unchanged_between_phases: bool,
    pub ledger_unchanged_between_phases: bool,
    pub file_gate_ready: bool,
    pub bao_verification_pending: bool,
    pub atomic_filesystem_snapshot: bool,
}

fn database_hash_bounded(path: &std::path::Path) -> Result<[u8; 32], JointHold> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    if size == 0 || size > MAX_DATABASE_BYTES {
        return Err(JointHold::DatabaseTooLarge);
    }
    let mut hasher = blake3::Hasher::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let len = file.read(&mut buffer)?;
        if len == 0 {
            break;
        }
        total = total
            .checked_add(len as u64)
            .ok_or(JointHold::DatabaseTooLarge)?;
        if total > MAX_DATABASE_BYTES {
            return Err(JointHold::DatabaseTooLarge);
        }
        hasher.update(&buffer[..len]);
    }
    if total != size {
        return Err(JointHold::DatabaseChangedBetweenPhases);
    }
    Ok(*hasher.finalize().as_bytes())
}

/// Sequential joined evidence with fail-closed discrepancy detection.
///
/// PRECONDITION: the pinned iroh-blobs store is shut down, and an independent
/// OS-enforced boundary prevents untrusted concurrent writes/replacement.
/// The ledger is exclusively held and the mount is checked before/after,
/// but pathname-based access here is NOT a hostile-race-safe storage grant.
pub fn audit_quiescent_staging(
    volume: &StagingVolumeGuard,
    ledger: &DurableLedger,
    store_child: &str,
) -> Result<JointEvidenceOnly, JointHold> {
    volume.check()?;
    // Ledger has one immutable snapshot for the duration of this shared borrow.
    let prior = ledger.snapshot()?;
    let db = audit_pinned_redb_manifest(volume, ledger, store_child)?;
    let file = audit_external_inventory_with_inline(volume, ledger, store_child, &db.inline_hashes)?;

    // Verify all three independent decisions refer to the same durable
    // revision, the same number of validated objects and the exact byte sum.
    let inline_count = db.inline_hashes.len();
    let expected_external = db
        .summary
        .blob_records
        .checked_sub(inline_count)
        .ok_or(JointHold::CrossComponentMismatch)?;
    let total = file
        .verified_external_bytes
        .checked_add(db.summary.verified_inline_bytes)
        .ok_or(JointHold::CrossComponentMismatch)?;

    if db.summary.file_gate_ready
        || file.file_gate_ready
        || !db.summary.bao_verification_pending
        || !db.summary.database_unchanged
        || db.summary.journal_generation != prior.generation
        || file.journal_generation != prior.generation
        || db.summary.blob_records != prior.objects
        || file.reserved_objects != prior.objects
        || file.reserved_bytes != prior.reserved_bytes
        || prior.objects != prior.verified_objects
        || file.verified_external_data != expected_external
        || inline_count != db.summary.inline_data_entries
        || total != prior.reserved_bytes
    {
        return Err(JointHold::CrossComponentMismatch);
    }

    // Existing 09b redb scan checked its byte digest before and after the DB
    // transaction. Re-hash AFTER scanning external files to detect database
    // change in between these non-atomic, sequential phases.
    let root = volume.root().join(store_child);
    if database_hash_bounded(&root.join("blobs.db"))? != db.database_digest {
        return Err(JointHold::DatabaseChangedBetweenPhases);
    }
    if ledger.snapshot()? != prior {
        return Err(JointHold::JournalChangedBetweenPhases);
    }
    volume.check()?;

    Ok(JointEvidenceOnly {
        ledger_generation: prior.generation,
        durable_blob_count: prior.objects,
        verified_external_blobs: file.verified_external_data,
        verified_inline_blobs: inline_count,
        verified_total_bytes: total,
        redb_tables_scanned: true,
        database_unchanged_between_phases: true,
        ledger_unchanged_between_phases: true,
        file_gate_ready: false,
        bao_verification_pending: true,
        atomic_filesystem_snapshot: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cannot_construct_file_gate_approval_from_evidence() {
        let evidence = JointEvidenceOnly {
            ledger_generation: 4,
            durable_blob_count: 1,
            verified_external_blobs: 1,
            verified_inline_blobs: 0,
            verified_total_bytes: 99,
            redb_tables_scanned: true,
            database_unchanged_between_phases: true,
            ledger_unchanged_between_phases: true,
            file_gate_ready: false,
            bao_verification_pending: true,
            atomic_filesystem_snapshot: false,
        };
        assert!(!evidence.file_gate_ready);
        assert!(!evidence.atomic_filesystem_snapshot);
        assert!(evidence.bao_verification_pending);
    }
}
