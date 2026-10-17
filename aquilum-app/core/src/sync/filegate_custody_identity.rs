//! Persistent forensic identity binding for one registered local vault and its
//! existing external OS lock. This database is APP DATA, never vault content.
//! It is not authenticated custody or a permission to mutate user files.
//! A public BLAKE3 checksum detects damage, NOT hostile same-user forgery.

use super::{filegate_lock_custody, RegisteredReadAdmission, VaultId, VaultPortError};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const SCHEMA: i64 = 1;

#[derive(Debug)]
pub(crate) enum CustodyBindingError {
    Admission(VaultPortError),
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    InvalidAppdata,
    MissingBinding,
    ConflictingIdentity,
    CorruptedBinding,
    UnsupportedSchema(i64),
    #[cfg(feature = "sync-s3")]
    MissingRegisteredS3,
}
impl From<VaultPortError> for CustodyBindingError {
    fn from(error: VaultPortError) -> Self { Self::Admission(error) }
}
impl From<rusqlite::Error> for CustodyBindingError {
    fn from(error: rusqlite::Error) -> Self { Self::Sqlite(error) }
}
impl From<std::io::Error> for CustodyBindingError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredCustodyIdentity {
    pub workspace_id: Uuid,
    pub vault_id: VaultId,
    pub registered_root_path: String,
    pub registered_root_volume: u64,
    pub registered_root_file: u64,
    pub custody_scope_path: String,
    pub custody_scope_volume: u64,
    pub custody_scope_file: u64,
    pub lock_volume: u64,
    pub lock_file: u64,
}

/// A persistent *diagnostic* record. The lifetime of the kernel lock does
/// not span this SQLite transaction; the database offers no file authority.
pub(crate) struct SqliteCustodyIdentityStore {
    connection: Connection,
    database_path: PathBuf,
}

impl SqliteCustodyIdentityStore {
    pub(crate) fn open(
        admission: &RegisteredReadAdmission<'_>,
        database_path: &Path,
    ) -> Result<Self, CustodyBindingError> {
        admission.recheck_registered_root()?;
        validate_store_location(admission, database_path)?;
        let connection = Connection::open(database_path)?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        match version {
            0 => connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE custody_anchor_binding (
                     workspace_id TEXT PRIMARY KEY NOT NULL,
                     vault_id TEXT UNIQUE NOT NULL,
                     registered_root_path TEXT UNIQUE NOT NULL,
                     record_json BLOB NOT NULL,
                     record_hash TEXT NOT NULL
                 );
                 PRAGMA user_version = 1;
                 COMMIT;",
            )?,
            SCHEMA => {},
            other => return Err(CustodyBindingError::UnsupportedSchema(other)),
        }
        // Reject an unrelated SQLite database masquerading as schema v1;
        // a matching PRAGMA user_version alone is never sufficient.
        connection.prepare("SELECT workspace_id FROM custody_anchor_binding LIMIT 0")?;
        validate_store_location(admission, database_path)?;
        admission.recheck_registered_root()?;
        Ok(Self { connection, database_path: database_path.to_path_buf() })
    }

    /// The expected VaultId must come from an independently reviewed, existing
    /// vault configuration. Supplying an arbitrary ID cannot make this record
    /// trusted. A different already recorded identity is never re-bound.
    pub(crate) fn bind_observed(
        &mut self,
        admission: &RegisteredReadAdmission<'_>,
        expected_vault: VaultId,
        existing_scope: &Path,
    ) -> Result<StoredCustodyIdentity, CustodyBindingError> {
        validate_store_location(admission, &self.database_path)?;
        let proposed = observe(admission, expected_vault, existing_scope)?;
        let json = serde_json::to_vec(&proposed)
            .map_err(|_| CustodyBindingError::CorruptedBinding)?;
        let digest = blake3::hash(&json).to_hex().to_string();
        let transaction = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = load(&transaction, proposed.workspace_id)? {
            if existing != proposed {
                return Err(CustodyBindingError::ConflictingIdentity);
            }
            transaction.commit()?;
        } else {
            // A collision with a different local root or VaultId fails via
            // SQLite UNIQUE constraints; never UPDATE/REPLACE a prior row.
            transaction.execute(
                "INSERT INTO custody_anchor_binding
                 (workspace_id, vault_id, registered_root_path, record_json, record_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    proposed.workspace_id.to_string(),
                    proposed.vault_id.to_string(),
                    &proposed.registered_root_path, json, digest
                ],
            )?;
            transaction.commit()?;
        }
        // A changed namespace after the transaction preserves the original
        // record, but does not return a positive observation.
        self.verify_observed(admission, expected_vault, existing_scope)?;
        Ok(proposed)
    }

    pub(crate) fn verify_observed(
        &self,
        admission: &RegisteredReadAdmission<'_>,
        expected_vault: VaultId,
        existing_scope: &Path,
    ) -> Result<StoredCustodyIdentity, CustodyBindingError> {
        validate_store_location(admission, &self.database_path)?;
        let existing = load(&self.connection, admission.registered_workspace_id())?
            .ok_or(CustodyBindingError::MissingBinding)?;
        if existing.vault_id != expected_vault
            || existing != observe(admission, expected_vault, existing_scope)?
        {
            return Err(CustodyBindingError::ConflictingIdentity);
        }
        validate_store_location(admission, &self.database_path)?;
        admission.recheck_registered_root()?;
        Ok(existing)
    }

    /// Only an existing, Core-registered S3 configuration supplies VaultId.
    /// A copied configuration or remote head is still NOT authenticated.
    #[cfg(feature = "sync-s3")]
    pub(crate) fn bind_registered_s3(
        &mut self,
        admission: &RegisteredReadAdmission<'_>,
        s3: &super::profile_binding::SqliteRegisteredS3Bindings,
        existing_scope: &Path,
    ) -> Result<StoredCustodyIdentity, CustodyBindingError> {
        let record = s3.load_for_registered(admission)
            .map_err(|_| CustodyBindingError::MissingRegisteredS3)?
            .ok_or(CustodyBindingError::MissingRegisteredS3)?;
        self.bind_observed(admission, record.vault_id, existing_scope)
    }

    #[cfg(feature = "sync-s3")]
    pub(crate) fn verify_registered_s3(
        &self,
        admission: &RegisteredReadAdmission<'_>,
        s3: &super::profile_binding::SqliteRegisteredS3Bindings,
        existing_scope: &Path,
    ) -> Result<StoredCustodyIdentity, CustodyBindingError> {
        let record = s3.load_for_registered(admission)
            .map_err(|_| CustodyBindingError::MissingRegisteredS3)?
            .ok_or(CustodyBindingError::MissingRegisteredS3)?;
        self.verify_observed(admission, record.vault_id, existing_scope)
    }
}

fn observe(
    admission: &RegisteredReadAdmission<'_>,
    vault_id: VaultId,
    scope: &Path,
) -> Result<StoredCustodyIdentity, CustodyBindingError> {
    admission.recheck_registered_root()?;
    let physical = admission.registered_root_identity_for_profile()?;
    let obs = admission.inspect_external_filegate_lock(scope)?;
    if obs.registered_root != physical {
        return Err(CustodyBindingError::ConflictingIdentity);
    }
    let registered_root_path = admission.registered_root_path().to_str()
        .ok_or(CustodyBindingError::InvalidAppdata)?.to_owned();
    // The external inspector separately refuses aliases/junctions and
    // rechecks every registered root while its kernel lock is held.
    let custody_scope_path = fs::canonicalize(scope)?.to_str()
        .ok_or(CustodyBindingError::InvalidAppdata)?.to_owned();
    admission.recheck_registered_root()?;
    Ok(StoredCustodyIdentity {
        workspace_id: admission.registered_workspace_id(),
        vault_id,
        registered_root_path,
        registered_root_volume: physical.0,
        registered_root_file: physical.1,
        custody_scope_path,
        custody_scope_volume: obs.custody_scope.0,
        custody_scope_file: obs.custody_scope.1,
        lock_volume: obs.lock_object.0,
        lock_file: obs.lock_object.1,
    })
}

fn load(
    db: &Connection,
    workspace: Uuid,
) -> Result<Option<StoredCustodyIdentity>, CustodyBindingError> {
    let row: Option<(String, String, Vec<u8>, String)> = db.query_row(
        "SELECT vault_id, registered_root_path, record_json, record_hash
         FROM custody_anchor_binding WHERE workspace_id = ?1",
        [workspace.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    ).optional()?;
    let Some((vault_id, root, json, digest)) = row else { return Ok(None); };
    if json.len() > 4096 || blake3::hash(&json).to_hex().as_str() != digest {
        return Err(CustodyBindingError::CorruptedBinding);
    }
    let binding: StoredCustodyIdentity = serde_json::from_slice(&json)
        .map_err(|_| CustodyBindingError::CorruptedBinding)?;
    if binding.workspace_id != workspace
        || binding.vault_id.to_string() != vault_id
        || binding.registered_root_path != root
    {
        return Err(CustodyBindingError::CorruptedBinding);
    }
    Ok(Some(binding))
}

/// SQLite is stored exclusively in app data, not in or above ANY registered
/// vault. This guards accidental reuse; it cannot defeat a hostile same-user
/// actor with namespace or SQLite write permissions.
fn validate_store_location(
    admission: &RegisteredReadAdmission<'_>,
    path: &Path,
) -> Result<(), CustodyBindingError> {
    admission.recheck_registered_root()?;
    let parent = path.parent().ok_or(CustodyBindingError::InvalidAppdata)?;
    if path.file_name().is_none_or(|leaf| leaf.is_empty()) || !path.is_absolute() {
        return Err(CustodyBindingError::InvalidAppdata);
    }
    let resolved_parent = fs::canonicalize(parent)?;
    filegate_lock_custody::require_no_alias_ancestors(&resolved_parent)?;
    let registered = admission.registered_workspace_roots()?;
    filegate_lock_custody::require_outside_registered_roots(&resolved_parent, &registered)?;
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.is_file() || meta.file_type().is_symlink() {
                return Err(CustodyBindingError::InvalidAppdata);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if meta.nlink() != 1 { return Err(CustodyBindingError::InvalidAppdata); }
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x0000_0400 != 0 {
                    return Err(CustodyBindingError::InvalidAppdata);
                }
                let opened = fs::File::open(path)?;
                let (_, _, links) = super::pinned_windows::file_identity_with_links(&opened)
                    .map_err(|_| CustodyBindingError::InvalidAppdata)?;
                if links != 1 { return Err(CustodyBindingError::InvalidAppdata); }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(CustodyBindingError::Io(error)),
    }
    Ok(())
}
