//! Opt-in host-ledger transition reservations for the rename laboratory.
//!
//! A reservation blocks *cooperating, installed-host* File Gate publications
//! and new host enrollment at BOTH source and destination names. It does not
//! authorize a rename, release a reservation, attest installer ownership,
//! secure same-user namespaces or fence default Core / S3 readers. There is
//! deliberately NO automatic expiry, replay, relocation or deletion.
//! The feature requires a freshly provisioned v2 ledger. Schema v1 is HOLD.
use super::{
    has_registered_path_overlap, HostCustodyError, PhysicalObjectId,
    SharedHostCustodyRegistry, MAX_RECORD_BYTES,
};
use crate::sync::filegate_lock_probe::PinnedProbeRoot;
use rusqlite::{params, Connection, Row, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const MAX_RESERVATIONS: usize = 512;

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostTransitionIntent {
    pub operation_id: Uuid,
    pub workspace_id: Uuid,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub source_identity: PhysicalObjectId,
}

fn invalid() -> HostCustodyError {
    HostCustodyError::InvalidRecord
}
fn damaged() -> HostCustodyError {
    HostCustodyError::DamagedRegistry
}
fn conflict() -> HostCustodyError {
    HostCustodyError::ConflictingEnrollment
}

fn dot_components(path: &Path) -> bool {
    path.to_string_lossy().replace('\\', "/").split('/')
        .any(|part| part == "." || part == "..")
}

fn overlaps(a: &Path, b: &Path) -> bool {
    crate::search::paths::strip_root(a, b).is_some()
        || crate::search::paths::strip_root(b, a).is_some()
}

impl HostTransitionIntent {
    fn valid_stored(&self) -> bool {
        if self.operation_id.is_nil() || self.workspace_id.is_nil()
            || !self.source_identity.valid()
            || !self.source.is_absolute() || !self.destination.is_absolute()
            || self.source.file_name().is_none() || self.destination.file_name().is_none()
            || self.source == self.destination
            || dot_components(&self.source) || dot_components(&self.destination)
            || overlaps(&self.source, &self.destination)
        {
            return false;
        }
        let (Some(a), Some(b)) = (self.source.parent(), self.destination.parent()) else {
            return false;
        };
        crate::search::paths::identity(a) == crate::search::paths::identity(b)
            && self.source.as_os_str().len() <= 2048
            && self.destination.as_os_str().len() <= 2048
    }

    fn check_live_source(&self) -> Result<(), HostCustodyError> {
        if !self.valid_stored() {
            return Err(invalid());
        }
        // Refuse symlink/junction ancestry, including source itself.
        super::super::filegate_lock_custody::require_no_alias_ancestors(&self.source)
            .map_err(|_| invalid())?;
        let physical_path = fs::canonicalize(&self.source)?;
        if crate::search::paths::identity(&physical_path)
            != crate::search::paths::identity(&self.source)
        {
            return Err(invalid());
        }
        let (device, file) = PinnedProbeRoot::open(&self.source)
            .map_err(|_| invalid())?.identity();
        if self.source_identity != (PhysicalObjectId { device, file }) {
            return Err(invalid());
        }
        let destination_parent = self.destination.parent().ok_or_else(invalid)?;
        super::super::filegate_lock_custody::require_no_alias_ancestors(destination_parent)
            .map_err(|_| invalid())?;
        match fs::symlink_metadata(&self.destination) {
            Ok(_) => Err(conflict()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(_) => Err(invalid()),
        }
    }
}

fn read_row(row: &Row<'_>) -> Result<HostTransitionIntent, HostCustodyError> {
    let operation: String = row.get(0)?;
    let workspace: String = row.get(1)?;
    let source: String = row.get(2)?;
    let destination: String = row.get(3)?;
    let root_key: String = row.get(4)?;
    let bytes: Vec<u8> = row.get(5)?;
    let digest: String = row.get(6)?;
    if bytes.is_empty() || bytes.len() > MAX_RECORD_BYTES
        || blake3::hash(&bytes).to_hex().as_str() != digest
    {
        return Err(damaged());
    }
    let record: HostTransitionIntent =
        serde_json::from_slice(&bytes).map_err(|_| damaged())?;
    if !record.valid_stored()
        || record.operation_id.to_string() != operation
        || record.workspace_id.to_string() != workspace
        || record.source.to_str() != Some(source.as_str())
        || record.destination.to_str() != Some(destination.as_str())
        || record.source_identity.key() != root_key
    {
        return Err(damaged());
    }
    Ok(record)
}

/// Full scan, never exact-key-only: corruption or an overlapping reservation
/// elsewhere must not silently disappear from an enrollment/write lookup.
pub(super) fn all(conn: &Connection) -> Result<Vec<HostTransitionIntent>, HostCustodyError> {
    let mut stmt = conn.prepare(
        "SELECT operation_id, workspace_id, source_path, destination_path,
                root_key, record_json, record_blake3 FROM workspace_transitions"
    )?;
    let mut rows = stmt.query([])?;
    let mut result = Vec::new();
    while let Some(row) = rows.next()? {
        if result.len() >= MAX_RESERVATIONS {
            return Err(damaged());
        }
        result.push(read_row(row)?);
    }
    Ok(result)
}

/// Used by enrollment and ordinary installed-host writer admission INSIDE
/// the SAME BEGIN IMMEDIATE SQLite transaction as their other host checks.
pub(super) fn reserved(
    conn: &Connection, root: &Path, identity: PhysicalObjectId,
) -> Result<bool, HostCustodyError> {
    Ok(all(conn)?.iter().any(|entry| {
        entry.source_identity == identity
            || overlaps(root, &entry.source)
            || overlaps(root, &entry.destination)
    }))
}

impl SharedHostCustodyRegistry {
    /// A test- and broker-facing primitive, not a runtime rename API.
    /// Caller must independently establish installer-controlled custody.
    /// No clear/replace/expiry operation is exposed, including after a crash.
    pub(crate) fn reserve_transition(
        &mut self, request: &HostTransitionIntent,
    ) -> Result<(), HostCustodyError> {
        request.check_live_source()?;
        self.identity.recheck()?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sidecars = self.identity.sidecars()?;
        self.identity.recheck()?;
        if has_registered_path_overlap(&tx, &request.source)?
            || has_registered_path_overlap(&tx, &request.destination)?
        {
            return Err(conflict());
        }
        if all(&tx)?.iter().any(|existing| {
            existing.operation_id == request.operation_id
                || existing.workspace_id == request.workspace_id
                || existing.source_identity == request.source_identity
                || overlaps(&existing.source, &request.source)
                || overlaps(&existing.source, &request.destination)
                || overlaps(&existing.destination, &request.source)
                || overlaps(&existing.destination, &request.destination)
        }) {
            return Err(conflict());
        }
        // Revalidate while holding the host DB writer epoch. This does not
        // defeat a malicious same-user path switch after validation.
        request.check_live_source()?;
        let bytes = serde_json::to_vec(request).map_err(|_| invalid())?;
        if bytes.is_empty() || bytes.len() > MAX_RECORD_BYTES {
            return Err(invalid());
        }
        let digest = blake3::hash(&bytes).to_hex().to_string();
        tx.execute(
            "INSERT INTO workspace_transitions (
                operation_id, workspace_id, source_path, destination_path,
                root_key, record_json, record_blake3
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                request.operation_id.to_string(), request.workspace_id.to_string(),
                request.source.to_string_lossy(), request.destination.to_string_lossy(),
                request.source_identity.key(), bytes, digest,
            ],
        )?;
        self.identity.recheck()?;
        if self.identity.sidecars()? != sidecars {
            return Err(HostCustodyError::InvalidHostLocation);
        }
        tx.commit()?;
        self.identity.recheck()?;
        Ok(())
    }

    /// Read-only inspection; never returns a mutation permit.
    pub(crate) fn transition_snapshot(
        &self,
    ) -> Result<Vec<HostTransitionIntent>, HostCustodyError> {
        self.identity.recheck()?;
        let items = all(&self.conn)?;
        self.identity.recheck()?;
        Ok(items)
    }
}

#[cfg(test)]
mod tests;
