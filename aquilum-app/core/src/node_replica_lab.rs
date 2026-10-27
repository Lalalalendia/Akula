//! NODE-E2E-01a: offline, operator-only replica proof using the existing
//! Sync DAG/blob store. NO Node service, network endpoint, owner enrollment,
//! File Gate capability, grant, or permission to write an original Vault.
//!
//! An operator-allowlisted D1.5 ReadCatalog is the only fixture source. The
//! existing SqliteDagStore is the sole blob/revision writer. A content blob
//! committed before its revision is NOT servable after a crash.
//! This lab does not protect against a malicious same-UID process replacing
//! SQLite files; production needs OS-provisioned Node custody and signed grants.

use crate::node_catalog_lab::{CatalogError, ReadCatalog};
use crate::sync::{
    DagStoreError, DocumentId, IngestOutcome, RevisionId, RevisionOperation,
    RevisionRecord, SqliteDagStore, SyncRelativePath, VaultId,
};
use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorFixtureAdmission {
    pub vault_id: VaultId,
    pub document_id: DocumentId,
    pub revision_id: RevisionId,
    pub filename: String,
    pub expected_blake3: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalReplicaObservation {
    pub vault_id: VaultId,
    pub document_id: DocumentId,
    pub revision_id: RevisionId,
    pub raw_blake3: String,
    pub byte_length: u64,
}

#[derive(Debug)]
pub enum ReplicaLabError {
    Io(io::Error),
    Catalog(CatalogError),
    Store(DagStoreError),
    InvalidAdmission,
    UnsafeLocation,
    UnsafeStore,
    ContentMismatch,
    MissingOrPendingRevision,
    ScopeMismatch,
}

impl From<io::Error> for ReplicaLabError {
    fn from(e: io::Error) -> Self { Self::Io(e) }
}
impl From<CatalogError> for ReplicaLabError {
    fn from(e: CatalogError) -> Self { Self::Catalog(e) }
}
impl From<DagStoreError> for ReplicaLabError {
    fn from(e: DagStoreError) -> Self { Self::Store(e) }
}

fn valid(admission: &OperatorFixtureAdmission) -> Result<SyncRelativePath, ReplicaLabError> {
    let hash = admission.expected_blake3.as_bytes();
    if admission.vault_id.as_uuid().is_nil()
        || admission.document_id.as_uuid().is_nil()
        || admission.revision_id.as_uuid().is_nil()
        || hash.len() != 64
        || !hash.iter().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
    {
        return Err(ReplicaLabError::InvalidAdmission);
    }
    let path = SyncRelativePath::parse(&admission.filename)
        .map_err(|_| ReplicaLabError::InvalidAdmission)?;
    if !path.as_str().ends_with(".md") || path.as_str().contains('/') {
        return Err(ReplicaLabError::InvalidAdmission);
    }
    Ok(path)
}

fn checked_state_dir(dir: &Path) -> Result<PathBuf, ReplicaLabError> {
    let meta = fs::symlink_metadata(dir).map_err(|_| ReplicaLabError::UnsafeLocation)?;
    if !meta.file_type().is_dir() || meta.uid() != rustix::process::geteuid().as_raw()
        || meta.permissions().mode() & 0o7777 != 0o700
    {
        return Err(ReplicaLabError::UnsafeLocation);
    }
    fs::canonicalize(dir).map_err(|_| ReplicaLabError::UnsafeLocation)
}

fn sqlite_path(dir: &Path, vault: VaultId) -> Result<PathBuf, ReplicaLabError> {
    let dir = checked_state_dir(dir)?;
    Ok(dir.join(format!("akula-node-replica-{vault}.sqlite")))
}

fn check_existing_file(path: &Path) -> Result<(), ReplicaLabError> {
    let meta = fs::symlink_metadata(path).map_err(|_| ReplicaLabError::UnsafeStore)?;
    if !meta.file_type().is_file() || meta.uid() != rustix::process::geteuid().as_raw()
        || meta.nlink() != 1 || meta.permissions().mode() & 0o7777 != 0o600
    {
        return Err(ReplicaLabError::UnsafeStore);
    }
    Ok(())
}

fn open_store(path: &Path, create: bool) -> Result<SqliteDagStore, ReplicaLabError> {
    match fs::symlink_metadata(path) {
        Ok(_) => check_existing_file(path)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound && create => {
            match OpenOptions::new().write(true).create_new(true).mode(0o600).open(path) {
                Ok(file) => drop(file),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {},
                Err(e) => return Err(ReplicaLabError::Io(e)),
            }
            check_existing_file(path)?;
        }
        Err(_) => return Err(ReplicaLabError::UnsafeStore),
    }
    Ok(SqliteDagStore::open(path)?)
}

fn verified(
    store: &SqliteDagStore,
    admission: &OperatorFixtureAdmission,
    path: &SyncRelativePath,
) -> Result<Vec<u8>, ReplicaLabError> {
    let dag = store.load_dag()?;
    let record = dag.get(admission.revision_id)
        .ok_or(ReplicaLabError::MissingOrPendingRevision)?;
    if record.document_id != admission.document_id || !record.parents.is_empty() {
        return Err(ReplicaLabError::ScopeMismatch);
    }
    match &record.operation {
        RevisionOperation::Content { path: recorded, blob_hash }
            if recorded == path && blob_hash == &admission.expected_blake3 => {}
        _ => return Err(ReplicaLabError::ScopeMismatch),
    }
    let bytes = store.verified_revision_content(admission.revision_id)?
        .ok_or(ReplicaLabError::MissingOrPendingRevision)?;
    if blake3::hash(&bytes).to_hex().as_str() != admission.expected_blake3 {
        return Err(ReplicaLabError::ContentMismatch);
    }
    Ok(bytes)
}

/// Read already staged bytes from a new process after reopening the same
/// operator-owned private SQLite file. Identifiers are NOT authorization.
/// Production MUST place DeviceId/owner-policy checks ahead of this call.
pub fn verify_local_replica(
    state_dir: &Path,
    admission: &OperatorFixtureAdmission,
) -> Result<Vec<u8>, ReplicaLabError> {
    let path = valid(admission)?;
    let db = sqlite_path(state_dir, admission.vault_id)?;
    let store = open_store(&db, false)?;
    verified(&store, admission, &path)
}

/// Operator-only, fixed-source fixture. The source root is never writable by
/// this API and the SQLite database must be in a separate private directory.
/// No directory is created, no network input is accepted, no File Gate bypass
/// exists, and a return value is NOT a signed durable owner receipt.
pub fn stage_operator_fixture(
    source_root: &Path,
    state_dir: &Path,
    admission: &OperatorFixtureAdmission,
) -> Result<LocalReplicaObservation, ReplicaLabError> {
    stage_operator_fixture_with_hook(source_root, state_dir, admission, || {})
}

/// Crash hook for a real subprocess test: executes after the existing
/// SqliteDagStore blob transaction but BEFORE the revision is published.
pub fn stage_operator_fixture_with_hook(
    source_root: &Path,
    state_dir: &Path,
    admission: &OperatorFixtureAdmission,
    after_blob_commit: impl FnOnce(),
) -> Result<LocalReplicaObservation, ReplicaLabError> {
    let path = valid(admission)?;
    let source = fs::canonicalize(source_root).map_err(|_| ReplicaLabError::UnsafeLocation)?;
    let state = checked_state_dir(state_dir)?;
    if source.starts_with(&state) || state.starts_with(&source) {
        return Err(ReplicaLabError::UnsafeLocation);
    }
    let catalog = ReadCatalog::open(source_root, admission.vault_id, &[path.as_str()])?;
    let object = catalog.list().into_iter().next()
        .ok_or(ReplicaLabError::InvalidAdmission)?;
    let snapshot = catalog.read(&object.id)?;
    if snapshot.raw_revision != admission.expected_blake3 {
        return Err(ReplicaLabError::ContentMismatch);
    }
    let bytes = snapshot.content.into_bytes();
    if blake3::hash(&bytes).to_hex().as_str() != admission.expected_blake3 {
        return Err(ReplicaLabError::ContentMismatch);
    }

    let db = sqlite_path(state_dir, admission.vault_id)?;
    let mut store = open_store(&db, true)?;
    let hash = store.store_blob(&bytes)?;
    if hash != admission.expected_blake3 { return Err(ReplicaLabError::ContentMismatch); }
    after_blob_commit();

    let record = RevisionRecord {
        id: admission.revision_id,
        document_id: admission.document_id,
        parents: Vec::new(),
        operation: RevisionOperation::Content { path: path.clone(), blob_hash: hash },
    };
    match store.ingest(record)? {
        IngestOutcome::Activated(_) | IngestOutcome::AlreadyPresent => {}
        IngestOutcome::Pending => return Err(ReplicaLabError::MissingOrPendingRevision),
    }
    let observed = verified(&store, admission, &path)?;
    if observed != bytes { return Err(ReplicaLabError::ContentMismatch); }
    Ok(LocalReplicaObservation {
        vault_id: admission.vault_id,
        document_id: admission.document_id,
        revision_id: admission.revision_id,
        raw_blake3: admission.expected_blake3.clone(),
        byte_length: observed.len() as u64,
    })
}
