//! P0-B6b-1: separate, previously issued installer witness for the host ledger.
//! This does NOT attest ownership or defend against a hostile same-user writer.
//! A real installer/broker must protect the witness *independently* of SQLite.
//! Never create/adopt a witness from the database being inspected at startup.

use super::filegate_lock_probe::PinnedProbeRoot;
use super::host_custody_registry::HostCustodyError;
use super::host_ledger_identity::{checked_member, HostLedgerIdentity, HostLedgerObjectIds};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const WITNESS_VERSION: u32 = 1;
const MAX_WITNESS_BYTES: usize = 2048;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstallWitness {
    version: u32,
    installation_id: uuid::Uuid,
    ledger_path: PathBuf,
    ledger_objects: HostLedgerObjectIds,
}

/// Observe only; neither this function nor Core auto-provisions, repairs or
/// accepts a new witness. The two physical namespaces must be distinct.
/// The expected IDs are read from a separate file, NOT derived from the
/// candidate SQLite database. Same-user tampering remains out of scope.
pub(crate) fn read_installer_witness(
    db_path: &Path, witness_path: &Path,
) -> Result<HostLedgerObjectIds, HostCustodyError> {
    if !db_path.is_absolute() || !witness_path.is_absolute()
        || db_path == witness_path
    {
        return Err(HostCustodyError::InvalidHostLocation);
    }
    let db_parent = db_path.parent().ok_or(HostCustodyError::InvalidHostLocation)?;
    let witness_parent = witness_path.parent()
        .ok_or(HostCustodyError::InvalidHostLocation)?;
    // Validate both immediate parents with the same symlink/reparse rejection
    // used for protected root pinning. This is not an ancestor/ACL attestation.
    let pinned_witness_parent = PinnedProbeRoot::open(witness_parent)?;
    let canonical_db_parent = fs::canonicalize(db_parent)?;
    let canonical_witness_parent = fs::canonicalize(witness_parent)?;
    if canonical_witness_parent.starts_with(&canonical_db_parent)
        || canonical_db_parent.starts_with(&canonical_witness_parent)
    {
        // A witness inside the mutable DB namespace is not independent.
        return Err(HostCustodyError::InvalidHostLocation);
    }
    let (file, first_id) = checked_member(witness_path)?
        .ok_or(HostCustodyError::InvalidHostLocation)?;
    let mut bytes = Vec::new();
    file.take((MAX_WITNESS_BYTES + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > MAX_WITNESS_BYTES {
        return Err(HostCustodyError::InvalidHostLocation);
    }
    // Readback of this distinct witness file is safe for Unix SQLite fcntl
    // locks: it is never the SQLite DB, WAL or SHM descriptor.
    let after_id = checked_member(witness_path)?.map(|(_, identity)| identity);
    if after_id != Some(first_id) {
        return Err(HostCustodyError::InvalidHostLocation);
    }
    pinned_witness_parent.recheck_root()?;
    let record: InstallWitness = serde_json::from_slice(&bytes)
        .map_err(|_| HostCustodyError::InvalidHostLocation)?;
    if record.version != WITNESS_VERSION
        || record.installation_id.is_nil()
        || record.ledger_path != db_path
        || record.ledger_objects.parent_file == 0
        || record.ledger_objects.db_file == 0
        || serde_json::to_vec(&record).ok().as_deref() != Some(bytes.as_slice())
    {
        return Err(HostCustodyError::InvalidHostLocation);
    }
    Ok(record.ledger_objects)
}

/// Disposable CI/test fixture. Never ship this as a recovery or installer
/// bootstrap: it deliberately derives a claim from the CURRENT DB.
#[cfg(test)]
pub(crate) fn create_test_witness_once(
    db_path: &Path, witness_path: &Path,
) -> Result<(), HostCustodyError> {
    use std::fs::OpenOptions;
    use std::io::Write;

    let expected = HostLedgerIdentity::capture(db_path)?.object_ids();
    let claim = InstallWitness {
        version: WITNESS_VERSION,
        installation_id: uuid::Uuid::new_v4(),
        ledger_path: db_path.to_path_buf(),
        ledger_objects: expected,
    };
    let bytes = serde_json::to_vec(&claim)
        .map_err(|_| HostCustodyError::InvalidHostLocation)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(witness_path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    assert_eq!(read_installer_witness(db_path, witness_path)?, expected);
    Ok(())
}
