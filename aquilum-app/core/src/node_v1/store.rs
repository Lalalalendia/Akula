//! Immutable Node replica over the existing SQLite Sync DAG. No Vault writer,
//! remote path, delete or unscoped PUT. All store calls require a sealed owner
//! authorization verified by RootTrust on the local Node process.

use super::{NodeV1Error, trust::AdmittedPut};
use super::put::MAX_PUT_BYTES;
use crate::sync::{
    DocumentId, IngestOutcome, RevisionId, RevisionOperation, RevisionRecord,
    SqliteDagStore, SyncRelativePath, VaultId,
};
use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredRevision {
    pub vault_id: VaultId,
    pub document_id: DocumentId,
    pub revision_id: RevisionId,
    pub raw_blake3: String,
    pub byte_length: u64,
}

fn db_path(state: &Path, vault: VaultId, create: bool)
    -> Result<PathBuf, NodeV1Error>
{
    let dir = fs::symlink_metadata(state)?;
    if !dir.is_dir() || dir.uid() != rustix::process::geteuid().as_raw()
        || dir.permissions().mode() & 0o7777 != 0o700
    {
        return Err(NodeV1Error::Hold("unsafe Node replica StateDirectory"));
    }
    let path = state.join(format!("akula-node-replica-{vault}.sqlite"));
    match fs::symlink_metadata(&path) {
        Ok(_) => {},
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => {
            match OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
                Ok(file) => {
                    file.sync_all()?;
                    File::open(state)?.sync_all()?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {},
                Err(e) => return Err(e.into()),
            }
        }
        Err(_) => return Err(NodeV1Error::Hold("Node replica is not provisioned")),
    }
    let meta = fs::symlink_metadata(&path)?;
    if !meta.is_file() || meta.uid() != rustix::process::geteuid().as_raw()
        || meta.nlink() != 1 || meta.permissions().mode() & 0o7777 != 0o600
    {
        return Err(NodeV1Error::Hold("unsafe Node replica database"));
    }
    // Same-UID process replacement is outside this beta's isolation model;
    // service StateDirectory must have a dedicated, nonshared user.
    Ok(path)
}

fn open_store(state: &Path, vault: VaultId, create: bool)
    -> Result<SqliteDagStore, NodeV1Error>
{
    let file = db_path(state, vault, create)?;
    SqliteDagStore::open(&file)
        .map_err(|_| NodeV1Error::Hold("Node replica database refused"))
}

fn verify(
    store: &SqliteDagStore, scope: &StoredRevision, path: &SyncRelativePath,
    check_length: bool,
) -> Result<Vec<u8>, NodeV1Error> {
    let dag = store.load_dag()
        .map_err(|_| NodeV1Error::Hold("invalid Node revision DAG"))?;
    let record = dag.get(scope.revision_id)
        .ok_or(NodeV1Error::Hold("Node revision missing or pending"))?;
    if record.document_id != scope.document_id || !record.parents.is_empty() {
        return Err(NodeV1Error::Hold("Node document/revision mismatch"));
    }
    match &record.operation {
        RevisionOperation::Content { path: recorded, blob_hash }
            if recorded == path && blob_hash == &scope.raw_blake3 => {}
        _ => return Err(NodeV1Error::Hold("Node revision is not exact content")),
    }
    let bytes = store.verified_revision_content(scope.revision_id)
        .map_err(|_| NodeV1Error::Hold("Node content integrity failure"))?
        .ok_or(NodeV1Error::Hold("Node revision has no content"))?;
    if (check_length && bytes.len() as u64 != scope.byte_length)
        || blake3::hash(&bytes).to_hex().as_str() != scope.raw_blake3
    {
        return Err(NodeV1Error::Hold("Node replica hash/length mismatch"));
    }
    Ok(bytes)
}

/// The full store is intentionally not public to a caller. All access must
/// follow independent signed owner policy and the one-use device proof.
pub(super) fn commit(
    state: &Path, admitted: &AdmittedPut, bytes: &[u8],
) -> Result<StoredRevision, NodeV1Error> {
    let grant = admitted.scope();
    if bytes.len() > MAX_PUT_BYTES || std::str::from_utf8(bytes).is_err() {
        return Err(NodeV1Error::Hold("Node replica is not bounded UTF-8 Markdown"));
    }
    let hash = blake3::hash(bytes).to_hex().to_string();
    if hash != grant.expected_blake3 {
        return Err(NodeV1Error::Hold("Node offered bytes do not match signed hash"));
    }
    let path = SyncRelativePath::parse(&grant.filename)
        .map_err(|_| NodeV1Error::Hold("invalid signed Node filename"))?;
    if path.as_str().contains('/') || !path.as_str().ends_with(".md") {
        return Err(NodeV1Error::Hold("Node only accepts exact top-level Markdown"));
    }
    let scope = StoredRevision {
        vault_id: grant.vault_id,
        document_id: grant.document_id,
        revision_id: grant.revision_id,
        raw_blake3: hash,
        byte_length: bytes.len() as u64,
    };

    let mut store = open_store(state, scope.vault_id, true)?;
    let committed_hash = store.store_blob(bytes)
        .map_err(|_| NodeV1Error::Hold("Node immutable blob commit failed"))?;
    if committed_hash != scope.raw_blake3 {
        return Err(NodeV1Error::Hold("Node blob digest disagreement"));
    }
    let record = RevisionRecord {
        id: scope.revision_id,
        document_id: scope.document_id,
        parents: vec![],
        operation: RevisionOperation::Content {
            path: path.clone(), blob_hash: committed_hash,
        },
    };
    match store.ingest(record)
        .map_err(|_| NodeV1Error::Hold("Node immutable revision commit failed"))?
    {
        IngestOutcome::Activated(_) | IngestOutcome::AlreadyPresent => {},
        IngestOutcome::Pending => {
            return Err(NodeV1Error::Hold("Node revision has unresolved parents"));
        }
    }
    drop(store);
    // A receipt is never based on the staging transaction's in-memory value.
    let reopened = open_store(state, scope.vault_id, false)?;
    if verify(&reopened, &scope, &path, true)? != bytes {
        return Err(NodeV1Error::Hold("Node cold readback mismatch"));
    }
    Ok(scope)
}

pub(super) fn read_verified(
    state: &Path, scope: &StoredRevision, filename: &str,
) -> Result<Vec<u8>, NodeV1Error> {
    let path = SyncRelativePath::parse(filename)
        .map_err(|_| NodeV1Error::Hold("invalid requested Node filename"))?;
    let store = open_store(state, scope.vault_id, false)?;
    verify(&store, scope, &path, false)
}
