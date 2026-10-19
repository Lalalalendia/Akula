//! RN-4C: LAB-ONLY, disabled-by-default, Core-registered P2P grant ledger.
//!
//! Separate SQLite in appdata. A registered workspace and its pinned physical
//! root are required for EVERY operation. No network API or File Gate writer
//! uses this module. It supplies only an explicitly host-provisioned policy
//! snapshot for read-only experimental admission.
//!
//! Security limits: no installer-owned OS custody/ACL, no hostile same-user
//! writer defense, no atomic revocation/write lease, no issuer private-key
//! custody, no power-loss proof, no canonical remote consent process.

use super::{DeviceId, RegisteredReadAdmission, VaultId, VaultPortError};
use crate::app_core::Core;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

const SCHEMA: i64 = 1;
pub const MAX_LAB_GRANT_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug)]
pub enum GrantLedgerError {
    Unregistered(VaultPortError),
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    MissingStore,
    UnsafeStore,
    InvalidSchema,
    RootMismatch,
    InvalidProposal,
    ActiveGrantExists,
    MissingGrant,
    Revoked,
    StaleEpoch,
    CorruptRow,
}

impl From<rusqlite::Error> for GrantLedgerError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}
impl From<std::io::Error> for GrantLedgerError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<VaultPortError> for GrantLedgerError {
    fn from(value: VaultPortError) -> Self {
        Self::Unregistered(value)
    }
}

type Result<T> = std::result::Result<T, GrantLedgerError>;

/// An issuance request must come from trusted host/application code, never
/// from a remote peer or an unverified QUIC manifest.
#[derive(Clone, Copy, Debug)]
pub struct ApprovedPull {
    pub vault_id: VaultId,
    pub device_id: DeviceId,
    pub authenticated_endpoint: [u8; 32],
    pub trusted_issuer: [u8; 32],
    pub object_blake3: [u8; 32],
    pub object_length: u64,
    pub expires_at: u64,
    pub nonce: [u8; 16],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoredPull {
    pub vault_id: VaultId,
    pub device_id: DeviceId,
    pub authenticated_endpoint: [u8; 32],
    pub trusted_issuer: [u8; 32],
    pub object_blake3: [u8; 32],
    pub object_length: u64,
    pub epoch: u64,
    pub expires_at: u64,
    pub nonce: [u8; 16],
}

/// Scoped by a live, pinned, registered Akula Core root. This object NEVER
/// returns a File Gate or network write capability. No Core mutation methods.
pub struct RegisteredPullGrantLedger<'a> {
    admission: RegisteredReadAdmission<'a>,
    db: Connection,
    db_path: PathBuf,
}

fn checked_db_location(admission: &RegisteredReadAdmission<'_>, path: &Path) -> Result<()> {
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(GrantLedgerError::UnsafeStore);
    }
    let parent = path.parent().ok_or(GrantLedgerError::UnsafeStore)?;
    let physical = fs::canonicalize(parent)?;
    for root in admission.registered_workspace_roots()? {
        if let Ok(root) = fs::canonicalize(root) {
            if physical.starts_with(root) {
                return Err(GrantLedgerError::UnsafeStore);
            }
        }
    }
    Ok(())
}

fn bytes<const N: usize>(raw: Vec<u8>) -> Result<[u8; N]> {
    raw.try_into().map_err(|_| GrantLedgerError::CorruptRow)
}

impl<'a> RegisteredPullGrantLedger<'a> {
    /// Explicit provisioning only. Missing database is NOT an empty roster
    /// during a normal cold-start open. Provision leaves forensic evidence
    /// on partial failures; it never silently deletes an existing database.
    pub fn provision_new(core: &'a Core, root: &Path, database: &Path) -> Result<Self> {
        let admission = RegisteredReadAdmission::open(core, root)?;
        checked_db_location(&admission, database)?;
        drop(OpenOptions::new().write(true).create_new(true).open(database)?);
        let mut db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        db.busy_timeout(std::time::Duration::from_secs(3))?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "synchronous", "FULL")?;
        let (dev, ino) = admission.registered_root_identity_for_profile()?;
        let workspace = admission.registered_workspace_id().to_string();
        let path = admission.registered_root_path().to_string_lossy().to_string();
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "CREATE TABLE root_binding(
                workspace TEXT NOT NULL, path TEXT NOT NULL,
                root_dev TEXT NOT NULL, root_ino TEXT NOT NULL
             );
             CREATE TABLE pull_grants(
                vault TEXT NOT NULL, device TEXT NOT NULL,
                endpoint BLOB NOT NULL, issuer BLOB NOT NULL,
                content BLOB NOT NULL, content_len INTEGER NOT NULL,
                epoch INTEGER NOT NULL, expiry INTEGER NOT NULL,
                nonce BLOB NOT NULL, active INTEGER NOT NULL,
                PRIMARY KEY(vault, device)
             );
             PRAGMA user_version=1;",
        )?;
        tx.execute(
            "INSERT INTO root_binding(workspace,path,root_dev,root_ino)
             VALUES(?1,?2,?3,?4)",
            params![workspace, path, dev.to_string(), ino.to_string()],
        )?;
        admission.recheck_registered_root()?;
        tx.commit()?;
        Ok(Self {
            admission,
            db,
            db_path: database.to_path_buf(),
        })
    }

    /// Opens existing, never creates; no "missing means empty" fallback.
    pub fn open_existing(core: &'a Core, root: &Path, database: &Path) -> Result<Self> {
        let admission = RegisteredReadAdmission::open(core, root)?;
        checked_db_location(&admission, database)?;
        let meta = match fs::symlink_metadata(database) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(GrantLedgerError::MissingStore);
            }
            Err(e) => return Err(e.into()),
        };
        if !meta.file_type().is_file() || meta.file_type().is_symlink() {
            return Err(GrantLedgerError::UnsafeStore);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.nlink() != 1 {
                return Err(GrantLedgerError::UnsafeStore);
            }
        }
        let db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        db.busy_timeout(std::time::Duration::from_secs(3))?;
        db.pragma_update(None, "synchronous", "FULL")?;
        let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != SCHEMA {
            return Err(GrantLedgerError::InvalidSchema);
        }
        let (dev, ino) = admission.registered_root_identity_for_profile()?;
        let stored: (String, String, String, String) = db
            .query_row(
                "SELECT workspace,path,root_dev,root_ino FROM root_binding",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|_| GrantLedgerError::InvalidSchema)?;
        if stored != (
            admission.registered_workspace_id().to_string(),
            admission.registered_root_path().to_string_lossy().to_string(),
            dev.to_string(),
            ino.to_string(),
        ) {
            return Err(GrantLedgerError::RootMismatch);
        }
        Ok(Self {
            admission,
            db,
            db_path: database.to_path_buf(),
        })
    }

    pub fn database_path(&self) -> &Path {
        &self.db_path
    }

    /// A matching active grant cannot be silently replaced. Revocation
    /// records remain as epoch tombstones, so a later grant uses epoch+1.
    pub fn issue(&mut self, req: ApprovedPull) -> Result<StoredPull> {
        self.admission.recheck_registered_root()?;
        if req.object_length > MAX_LAB_GRANT_BYTES
            || req.expires_at == 0
            || req.authenticated_endpoint == [0; 32]
            || req.trusted_issuer == [0; 32]
            || req.nonce == [0; 16]
        {
            return Err(GrantLedgerError::InvalidProposal);
        }
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: Option<(i64, i64)> = tx
            .query_row(
                "SELECT epoch,active FROM pull_grants WHERE vault=?1 AND device=?2",
                params![req.vault_id.to_string(), req.device_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let epoch = match previous {
            Some((_, 1)) => return Err(GrantLedgerError::ActiveGrantExists),
            Some((old, 0)) if old >= 1 => old.checked_add(1)
                .ok_or(GrantLedgerError::CorruptRow)?,
            None => 1,
            _ => return Err(GrantLedgerError::CorruptRow),
        };
        tx.execute(
            "INSERT INTO pull_grants(vault,device,endpoint,issuer,content,content_len,epoch,expiry,nonce,active)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,1)
             ON CONFLICT(vault,device) DO UPDATE SET
               endpoint=excluded.endpoint, issuer=excluded.issuer,
               content=excluded.content, content_len=excluded.content_len,
               epoch=excluded.epoch, expiry=excluded.expiry,
               nonce=excluded.nonce, active=1",
            params![
                req.vault_id.to_string(), req.device_id.to_string(),
                &req.authenticated_endpoint, &req.trusted_issuer,
                &req.object_blake3, i64::try_from(req.object_length).unwrap(),
                epoch, i64::try_from(req.expires_at).map_err(|_| GrantLedgerError::InvalidProposal)?,
                &req.nonce,
            ],
        )?;
        self.admission.recheck_registered_root()?;
        tx.commit()?;
        Ok(StoredPull {
            vault_id: req.vault_id, device_id: req.device_id,
            authenticated_endpoint: req.authenticated_endpoint,
            trusted_issuer: req.trusted_issuer,
            object_blake3: req.object_blake3, object_length: req.object_length,
            epoch: epoch as u64, expires_at: req.expires_at, nonce: req.nonce,
        })
    }

    /// Read-only snapshot, NOT a lease and NOT authority to publish a file.
    /// A caller must revalidate immediately before using real bytes.
    pub fn read_active(&self, vault: VaultId, device: DeviceId, now: u64) -> Result<StoredPull> {
        self.admission.recheck_registered_root()?;
        let raw: Option<(Vec<u8>,Vec<u8>,Vec<u8>,i64,i64,i64,Vec<u8>,i64)> = self.db
            .query_row(
                "SELECT endpoint,issuer,content,content_len,epoch,expiry,nonce,active
                 FROM pull_grants WHERE vault=?1 AND device=?2",
                params![vault.to_string(), device.to_string()],
                |row| Ok((
                    row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?,
                    row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?,
                )),
            )
            .optional()?;
        let (endpoint,issuer,content,content_len,epoch,expiry,nonce,active) =
            raw.ok_or(GrantLedgerError::MissingGrant)?;
        if active == 0 {
            return Err(GrantLedgerError::Revoked);
        }
        if active != 1 || epoch <= 0 || content_len < 0 || expiry <= 0 {
            return Err(GrantLedgerError::CorruptRow);
        }
        let endpoint = bytes(endpoint)?;
        let issuer = bytes(issuer)?;
        let content = bytes(content)?;
        let nonce = bytes(nonce)?;
        if content_len as u64 > MAX_LAB_GRANT_BYTES || now >= expiry as u64 {
            return Err(GrantLedgerError::InvalidProposal);
        }
        self.admission.recheck_registered_root()?;
        Ok(StoredPull {
            vault_id: vault, device_id: device,
            authenticated_endpoint: endpoint, trusted_issuer: issuer,
            object_blake3: content, object_length: content_len as u64,
            epoch: epoch as u64, expires_at: expiry as u64, nonce,
        })
    }

    /// Atomic compare-and-set revocation, monotonic epoch tombstone.
    /// A stale caller cannot undo a concurrent revocation or re-enrollment.
    pub fn revoke(&mut self, vault: VaultId, device: DeviceId, expected_epoch: u64) -> Result<u64> {
        self.admission.recheck_registered_root()?;
        let expected = i64::try_from(expected_epoch).map_err(|_| GrantLedgerError::StaleEpoch)?;
        let next = expected.checked_add(1).ok_or(GrantLedgerError::StaleEpoch)?;
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE pull_grants SET epoch=?1,active=0
             WHERE vault=?2 AND device=?3 AND epoch=?4 AND active=1",
            params![next, vault.to_string(), device.to_string(), expected],
        )?;
        if changed != 1 {
            return Err(GrantLedgerError::StaleEpoch);
        }
        self.admission.recheck_registered_root()?;
        tx.commit()?;
        Ok(next as u64)
    }
}
