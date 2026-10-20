//! AKAI-AUTH-01b0: host-local, default-deny MCP agent/vault read grants.
//!
//! INERT FOUNDATION. No MCP dispatcher invokes this store. In particular,
//! an HTTP agent credential plus an approved grant is NOT sufficient to read
//! any file without host-pinned vault registration and per-resource routing.
//! The host must never construct a registration from untrusted MCP JSON.
//! This store is deliberately separate from package/plugin grants.

#![allow(dead_code)] // Activation requires the 01b trusted vault resolver.

use super::agent_principals::{AgentPrincipalId, AuthenticatedAgent};
use crate::plugins::{Capability, GrantScope, VaultId};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA_VERSION: i64 = 1;
const WHOLE_VAULT: &str = "whole_vault";
const BINDING_SIZE: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentGrantError {
    Denied,
    InvalidBinding,
    InvalidExpiry,
    StaleEpoch,
    ExhaustedEpoch,
    StoreUnavailable,
    CorruptStore,
    UnsupportedSchema,
}

impl fmt::Display for AgentGrantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // No user path, workspace name, agent ID or grant metadata in errors.
        write!(f, "MCP agent grant operation failed: {self:?}")
    }
}

impl std::error::Error for AgentGrantError {}

/// This is a host-issued *registration receipt*, not an authentication token.
/// `binding_digest` MUST come from a separately verified pinned host custody
/// record (physical root + installer authority), NOT a path or MCP argument.
/// The constructor validates format only. The trusted custody integration
/// does not exist yet, so this type must not be accepted by live MCP tools.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HostVaultRegistration {
    vault: VaultId,
    binding_digest: [u8; BINDING_SIZE],
}

impl HostVaultRegistration {
    /// TEST-ONLY self-issued receipt used by the isolated unit fixtures.
    /// A valid-looking binding digest is NOT a trusted installer/Host Broker
    /// attestation. Production has intentionally no receipt constructor until
    /// an independently authenticated, OS-protected issuer is integrated.
    #[cfg(test)]
    pub(crate) fn from_host_custody(
        vault: VaultId,
        binding_digest: [u8; BINDING_SIZE],
    ) -> Result<Self, AgentGrantError> {
        if binding_digest.iter().all(|byte| *byte == 0) {
            return Err(AgentGrantError::InvalidBinding);
        }
        Ok(Self { vault, binding_digest })
    }

    pub(crate) fn vault_id(self) -> VaultId {
        self.vault
    }
}

/// Snapshot of a persisted vault grant. It does not pin a physical file,
/// validate current credentials or reserve any File Gate commit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AgentReadLease {
    agent: AuthenticatedAgent,
    vault: VaultId,
    grant_epoch: u64,
}

pub(crate) struct AgentVaultGrantStore {
    db: Connection,
}

impl AgentVaultGrantStore {
    /// TRUSTED HOST ONLY: create a new database in a privately provisioned
    /// app-data directory. Never create/repair it implicitly on a read path.
    /// The installer remains responsible for protecting parent/sidecar ACLs.
    pub(crate) fn provision_new(path: &Path) -> Result<Self, AgentGrantError> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|_| AgentGrantError::StoreUnavailable)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| AgentGrantError::StoreUnavailable)?;
        }
        drop(file);
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|_| AgentGrantError::StoreUnavailable)?;
        configure(&db)?;
        db.execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE host_vault_registrations (
                 vault_id TEXT PRIMARY KEY NOT NULL,
                 binding_digest BLOB NOT NULL UNIQUE CHECK(length(binding_digest)=32)
             );
             CREATE TABLE agent_vault_epochs (
                 agent_id TEXT NOT NULL,
                 vault_id TEXT NOT NULL,
                 epoch INTEGER NOT NULL CHECK(epoch >= 0),
                 PRIMARY KEY(agent_id, vault_id),
                 FOREIGN KEY(vault_id) REFERENCES host_vault_registrations(vault_id)
                     ON DELETE RESTRICT
             );
             CREATE TABLE agent_vault_grants (
                 agent_id TEXT NOT NULL,
                 vault_id TEXT NOT NULL,
                 capability TEXT NOT NULL,
                 scope TEXT NOT NULL CHECK(scope='whole_vault'),
                 expires_at INTEGER,
                 PRIMARY KEY(agent_id, vault_id, capability),
                 FOREIGN KEY(agent_id, vault_id)
                     REFERENCES agent_vault_epochs(agent_id, vault_id)
                     ON DELETE RESTRICT
             );
             CREATE TABLE agent_vault_audit (
                 seq INTEGER PRIMARY KEY AUTOINCREMENT,
                 event TEXT NOT NULL CHECK(event IN ('grant_read', 'revoke_read')),
                 agent_id TEXT NOT NULL,
                 vault_id TEXT NOT NULL,
                 epoch INTEGER NOT NULL CHECK(epoch >= 0),
                 recorded_at INTEGER NOT NULL
             );
             PRAGMA user_version=1;
             COMMIT;",
        )
        .map_err(|_| AgentGrantError::StoreUnavailable)?;
        Ok(Self { db })
    }

    /// A missing database is a security failure, never an empty ACL.
    pub(crate) fn open_existing(path: &Path) -> Result<Self, AgentGrantError> {
        let info = fs::symlink_metadata(path).map_err(|_| AgentGrantError::StoreUnavailable)?;
        if !info.is_file() || info.file_type().is_symlink() {
            return Err(AgentGrantError::StoreUnavailable);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if info.file_attributes() & 0x400 != 0 {
                return Err(AgentGrantError::StoreUnavailable);
            }
        }
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|_| AgentGrantError::StoreUnavailable)?;
        configure(&db)?;
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|_| AgentGrantError::StoreUnavailable)?;
        if version != SCHEMA_VERSION {
            return Err(AgentGrantError::UnsupportedSchema);
        }
        for name in [
            "host_vault_registrations",
            "agent_vault_epochs",
            "agent_vault_grants",
            "agent_vault_audit",
        ] {
            let exists: i64 = db
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [name],
                    |row| row.get(0),
                )
                .map_err(|_| AgentGrantError::CorruptStore)?;
            if exists != 1 {
                return Err(AgentGrantError::CorruptStore);
            }
        }
        let integrity: String = db
            .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
            .map_err(|_| AgentGrantError::CorruptStore)?;
        if integrity != "ok" {
            return Err(AgentGrantError::CorruptStore);
        }
        Ok(Self { db })
    }

    /// The host may register a verified root identity once. The same VaultId
    /// cannot subsequently be rebound to another root without an error.
    /// Registration grants NO agent authority by itself.
    pub(crate) fn register_host_vault(
        &mut self,
        registration: HostVaultRegistration,
    ) -> Result<(), AgentGrantError> {
        let tx = self.db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AgentGrantError::StoreUnavailable)?;
        tx.execute(
            "INSERT OR IGNORE INTO host_vault_registrations(vault_id,binding_digest)
             VALUES(?1,?2)",
            params![registration.vault.as_uuid().to_string(), registration.binding_digest.as_slice()],
        ).map_err(|_| AgentGrantError::StoreUnavailable)?;
        require_registration(&tx, registration)?;
        tx.commit().map_err(|_| AgentGrantError::StoreUnavailable)?;
        Ok(())
    }

    /// A host-side CAS grant. Only the canonical plugin Capability vocabulary
    /// and WholeVault scope are used; no new string-based agent capability.
    /// The caller MUST separately authenticate the principal and verify its
    /// host custody registration; this module grants no live HTTP access.
    pub(crate) fn grant_notes_read(
        &mut self,
        principal: AgentPrincipalId,
        registration: HostVaultRegistration,
        capability: Capability,
        scope: GrantScope,
        expected_epoch: u64,
        expires_at: Option<i64>,
    ) -> Result<u64, AgentGrantError> {
        if capability != Capability::NotesRead || scope != GrantScope::WholeVault {
            return Err(AgentGrantError::Denied);
        }
        let timestamp = now()?;
        if expires_at.is_some_and(|value| value <= timestamp) {
            return Err(AgentGrantError::InvalidExpiry);
        }
        let tx = self.db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AgentGrantError::StoreUnavailable)?;
        require_registration(&tx, registration)?;
        let current = current_epoch(&tx, principal, registration.vault)?;
        if current != expected_epoch {
            return Err(AgentGrantError::StaleEpoch);
        }
        let next = increment(current)?;
        tx.execute(
            "INSERT OR IGNORE INTO agent_vault_epochs(agent_id,vault_id,epoch)
             VALUES(?1,?2,0)",
            params![principal.as_uuid().to_string(), registration.vault.as_uuid().to_string()],
        ).map_err(|_| AgentGrantError::StoreUnavailable)?;
        tx.execute(
            "UPDATE agent_vault_epochs SET epoch=?3 WHERE agent_id=?1 AND vault_id=?2",
            params![
                principal.as_uuid().to_string(),
                registration.vault.as_uuid().to_string(),
                to_sql_epoch(next)?,
            ],
        ).map_err(|_| AgentGrantError::StoreUnavailable)?;
        let capability = notes_read_key()?;
        tx.execute(
            "INSERT INTO agent_vault_grants(agent_id,vault_id,capability,scope,expires_at)
             VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(agent_id,vault_id,capability)
             DO UPDATE SET scope=excluded.scope, expires_at=excluded.expires_at",
            params![
                principal.as_uuid().to_string(),
                registration.vault.as_uuid().to_string(),
                capability,
                WHOLE_VAULT,
                expires_at,
            ],
        ).map_err(|_| AgentGrantError::StoreUnavailable)?;
        audit(&tx, "grant_read", principal, registration.vault, next, timestamp)?;
        tx.commit().map_err(|_| AgentGrantError::StoreUnavailable)?;
        Ok(next)
    }

    /// Revocation increments a durable epoch even when a grant was already
    /// expired. Previously handed-out leases become stale on the next check.
    pub(crate) fn revoke_notes_read(
        &mut self,
        principal: AgentPrincipalId,
        registration: HostVaultRegistration,
        expected_epoch: u64,
    ) -> Result<u64, AgentGrantError> {
        let timestamp = now()?;
        let tx = self.db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AgentGrantError::StoreUnavailable)?;
        require_registration(&tx, registration)?;
        let current = current_epoch(&tx, principal, registration.vault)?;
        if current == 0 || current != expected_epoch {
            return Err(AgentGrantError::StaleEpoch);
        }
        let next = increment(current)?;
        tx.execute(
            "DELETE FROM agent_vault_grants
             WHERE agent_id=?1 AND vault_id=?2 AND capability=?3",
            params![
                principal.as_uuid().to_string(),
                registration.vault.as_uuid().to_string(),
                notes_read_key()?,
            ],
        ).map_err(|_| AgentGrantError::StoreUnavailable)?;
        tx.execute(
            "UPDATE agent_vault_epochs SET epoch=?3 WHERE agent_id=?1 AND vault_id=?2",
            params![
                principal.as_uuid().to_string(),
                registration.vault.as_uuid().to_string(),
                to_sql_epoch(next)?,
            ],
        ).map_err(|_| AgentGrantError::StoreUnavailable)?;
        audit(&tx, "revoke_read", principal, registration.vault, next, timestamp)?;
        tx.commit().map_err(|_| AgentGrantError::StoreUnavailable)?;
        Ok(next)
    }

    /// Checked against this isolated registry only. An authorized result
    /// MUST NOT be used to return bytes until host physical-root and current
    /// credential-epoch validation exist at the read boundary.
    pub(crate) fn authorize_notes_read(
        &self,
        agent: AuthenticatedAgent,
        registration: HostVaultRegistration,
    ) -> Result<AgentReadLease, AgentGrantError> {
        require_registration(&self.db, registration)?;
        let record: Option<(i64, String, String, Option<i64>)> = self.db
            .query_row(
                "SELECT e.epoch, g.capability, g.scope, g.expires_at
                 FROM agent_vault_epochs e
                 JOIN agent_vault_grants g ON e.agent_id=g.agent_id AND e.vault_id=g.vault_id
                 WHERE e.agent_id=?1 AND e.vault_id=?2 AND g.capability=?3",
                params![
                    agent.principal.as_uuid().to_string(),
                    registration.vault.as_uuid().to_string(),
                    notes_read_key()?,
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|_| AgentGrantError::StoreUnavailable)?;
        let Some((epoch, capability, scope, expires_at)) = record else {
            return Err(AgentGrantError::Denied);
        };
        if serde_json::from_str::<Capability>(&capability).ok() != Some(Capability::NotesRead)
            || scope != WHOLE_VAULT
        {
            return Err(AgentGrantError::CorruptStore);
        }
        if expires_at.is_some_and(|expiry| expiry <= now().unwrap_or(i64::MAX)) {
            return Err(AgentGrantError::Denied);
        }
        if epoch <= 0 {
            return Err(AgentGrantError::CorruptStore);
        }
        Ok(AgentReadLease {
            agent,
            vault: registration.vault,
            grant_epoch: u64::try_from(epoch).map_err(|_| AgentGrantError::CorruptStore)?,
        })
    }

    /// Re-checks durable policy and binding; does NOT recheck agent secret
    /// revocation. The future router must also revalidate credential epoch.
    pub(crate) fn revalidate_read_lease(
        &self,
        lease: &AgentReadLease,
        agent: AuthenticatedAgent,
        registration: HostVaultRegistration,
    ) -> Result<(), AgentGrantError> {
        if lease.agent != agent || lease.vault != registration.vault {
            return Err(AgentGrantError::Denied);
        }
        let fresh = self.authorize_notes_read(agent, registration)?;
        if fresh.grant_epoch != lease.grant_epoch {
            return Err(AgentGrantError::StaleEpoch);
        }
        Ok(())
    }
}

fn configure(db: &Connection) -> Result<(), AgentGrantError> {
    db.execute_batch(
        "PRAGMA busy_timeout=2000;
         PRAGMA foreign_keys=ON;
         PRAGMA journal_mode=WAL;
         PRAGMA synchronous=FULL;",
    ).map_err(|_| AgentGrantError::StoreUnavailable)
}

fn require_registration(
    db: &Connection,
    receipt: HostVaultRegistration,
) -> Result<(), AgentGrantError> {
    let registered: Option<Vec<u8>> = db
        .query_row(
            "SELECT binding_digest FROM host_vault_registrations WHERE vault_id=?1",
            [receipt.vault.as_uuid().to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| AgentGrantError::StoreUnavailable)?;
    match registered {
        Some(value) if value.as_slice() == receipt.binding_digest.as_slice() => Ok(()),
        _ => Err(AgentGrantError::InvalidBinding),
    }
}

fn current_epoch(
    db: &Connection,
    principal: AgentPrincipalId,
    vault: VaultId,
) -> Result<u64, AgentGrantError> {
    let epoch: Option<i64> = db
        .query_row(
            "SELECT epoch FROM agent_vault_epochs WHERE agent_id=?1 AND vault_id=?2",
            params![principal.as_uuid().to_string(), vault.as_uuid().to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| AgentGrantError::StoreUnavailable)?;
    u64::try_from(epoch.unwrap_or(0)).map_err(|_| AgentGrantError::CorruptStore)
}

fn notes_read_key() -> Result<String, AgentGrantError> {
    serde_json::to_string(&Capability::NotesRead).map_err(|_| AgentGrantError::CorruptStore)
}

fn increment(epoch: u64) -> Result<u64, AgentGrantError> {
    let next = epoch.checked_add(1).ok_or(AgentGrantError::ExhaustedEpoch)?;
    to_sql_epoch(next)?;
    Ok(next)
}

fn to_sql_epoch(epoch: u64) -> Result<i64, AgentGrantError> {
    i64::try_from(epoch).map_err(|_| AgentGrantError::ExhaustedEpoch)
}

fn now() -> Result<i64, AgentGrantError> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|_| AgentGrantError::StoreUnavailable)?;
    i64::try_from(elapsed.as_secs()).map_err(|_| AgentGrantError::StoreUnavailable)
}

fn audit(
    db: &Connection,
    event: &str,
    principal: AgentPrincipalId,
    vault: VaultId,
    epoch: u64,
    timestamp: i64,
) -> Result<(), AgentGrantError> {
    db.execute(
        "INSERT INTO agent_vault_audit(event,agent_id,vault_id,epoch,recorded_at)
         VALUES(?1,?2,?3,?4,?5)",
        params![
            event,
            principal.as_uuid().to_string(),
            vault.as_uuid().to_string(),
            to_sql_epoch(epoch)?,
            timestamp,
        ],
    ).map_err(|_| AgentGrantError::StoreUnavailable)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::agent_principals::AgentCredentialStore;
    use tempfile::tempdir;

    fn issued(
        store: &mut AgentCredentialStore,
        label: &str,
    ) -> AuthenticatedAgent {
        let mut credential = store.issue_principal(label, None).unwrap();
        let token = credential.take_bearer_once().unwrap();
        store.authenticate(token.as_str()).unwrap()
    }

    fn registered(marker: u8) -> HostVaultRegistration {
        HostVaultRegistration::from_host_custody(VaultId::new_for_host(), [marker; 32]).unwrap()
    }

    #[test]
    fn two_agents_two_registered_vaults_cannot_cross_read() {
        let dir = tempdir().unwrap();
        let mut credentials =
            AgentCredentialStore::open(&dir.path().join("credentials.sqlite3")).unwrap();
        let a = issued(&mut credentials, "A");
        let b = issued(&mut credentials, "B");
        let x = registered(11);
        let y = registered(22);

        let mut grants =
            AgentVaultGrantStore::provision_new(&dir.path().join("grants.sqlite3")).unwrap();
        let never_enrolled = registered(33);
        assert_eq!(
            grants.grant_notes_read(
                a.principal,
                never_enrolled,
                Capability::NotesRead,
                GrantScope::WholeVault,
                0,
                None,
            ),
            Err(AgentGrantError::InvalidBinding)
        );
        grants.register_host_vault(x).unwrap();
        grants.register_host_vault(y).unwrap();
        assert_eq!(
            grants.grant_notes_read(
                a.principal, x, Capability::NotesPatch, GrantScope::WholeVault, 0, None
            ),
            Err(AgentGrantError::Denied)
        );
        assert_eq!(
            grants.grant_notes_read(
                a.principal, x, Capability::NotesRead, GrantScope::Action, 0, None
            ),
            Err(AgentGrantError::Denied)
        );
        assert_eq!(grants.authorize_notes_read(a, x), Err(AgentGrantError::Denied));
        assert_eq!(grants.authorize_notes_read(b, y), Err(AgentGrantError::Denied));
        assert_eq!(grants.grant_notes_read(a.principal, x, Capability::NotesRead, GrantScope::WholeVault, 0, None), Ok(1));
        assert_eq!(grants.grant_notes_read(b.principal, y, Capability::NotesRead, GrantScope::WholeVault, 0, None), Ok(1));

        assert!(grants.authorize_notes_read(a, x).is_ok());
        assert!(grants.authorize_notes_read(b, y).is_ok());
        assert_eq!(grants.authorize_notes_read(a, y), Err(AgentGrantError::Denied));
        assert_eq!(grants.authorize_notes_read(b, x), Err(AgentGrantError::Denied));

        let forged = HostVaultRegistration::from_host_custody(x.vault, [99; 32]).unwrap();
        assert_eq!(grants.authorize_notes_read(a, forged), Err(AgentGrantError::InvalidBinding));
        assert_eq!(grants.register_host_vault(forged), Err(AgentGrantError::InvalidBinding));
        let duplicate_root = HostVaultRegistration::from_host_custody(
            VaultId::new_for_host(),
            x.binding_digest,
        ).unwrap();
        assert_eq!(
            grants.register_host_vault(duplicate_root),
            Err(AgentGrantError::InvalidBinding),
            "the same physical binding must never be enrolled as a second vault"
        );
        assert_eq!(grants.authorize_notes_read(a, x).unwrap().vault, x.vault);
    }

    #[test]
    fn revoked_grant_is_durable_and_old_lease_is_stale() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("grants.sqlite3");
        let mut credentials =
            AgentCredentialStore::open(&dir.path().join("credentials.sqlite3")).unwrap();
        let agent = issued(&mut credentials, "Reader");
        let root = registered(17);

        let mut grants = AgentVaultGrantStore::provision_new(&path).unwrap();
        grants.register_host_vault(root).unwrap();
        let epoch = grants.grant_notes_read(agent.principal, root, Capability::NotesRead, GrantScope::WholeVault, 0, None).unwrap();
        let lease = grants.authorize_notes_read(agent, root).unwrap();
        assert_eq!(grants.revalidate_read_lease(&lease, agent, root), Ok(()));
        let next = grants.revoke_notes_read(agent.principal, root, epoch).unwrap();
        assert!(next > epoch);
        assert_eq!(
            grants.revalidate_read_lease(&lease, agent, root),
            Err(AgentGrantError::Denied)
        );
        drop(grants);

        let reopened = AgentVaultGrantStore::open_existing(&path).unwrap();
        assert_eq!(reopened.authorize_notes_read(agent, root), Err(AgentGrantError::Denied));
    }

    #[test]
    fn cas_epoch_and_expiry_fail_closed() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("grants.sqlite3");
        let mut credentials =
            AgentCredentialStore::open(&dir.path().join("credentials.sqlite3")).unwrap();
        let agent = issued(&mut credentials, "Short-lived");
        let root = registered(9);
        let mut first = AgentVaultGrantStore::provision_new(&path).unwrap();
        first.register_host_vault(root).unwrap();
        let mut second = AgentVaultGrantStore::open_existing(&path).unwrap();
        let expiry = now().unwrap() + 60;
        assert_eq!(
            first.grant_notes_read(agent.principal, root, Capability::NotesRead, GrantScope::WholeVault, 0, Some(expiry)),
            Ok(1)
        );
        assert_eq!(
            second.grant_notes_read(agent.principal, root, Capability::NotesRead, GrantScope::WholeVault, 0, None),
            Err(AgentGrantError::StaleEpoch)
        );
        let old = first.authorize_notes_read(agent, root).unwrap();
        assert_eq!(second.grant_notes_read(agent.principal, root, Capability::NotesRead, GrantScope::WholeVault, 1, None), Ok(2));
        assert_eq!(
            first.revalidate_read_lease(&old, agent, root),
            Err(AgentGrantError::StaleEpoch)
        );
        first.db.execute(
            "UPDATE agent_vault_grants SET expires_at=1",
            [],
        ).unwrap();
        assert_eq!(second.authorize_notes_read(agent, root), Err(AgentGrantError::Denied));
    }

    #[test]
    fn missing_database_wrong_schema_and_corrupt_registration_reject() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("grants.sqlite3");
        assert!(matches!(
            AgentVaultGrantStore::open_existing(&path),
            Err(AgentGrantError::StoreUnavailable)
        ));
        let mut grants = AgentVaultGrantStore::provision_new(&path).unwrap();
        let root = registered(42);
        grants.register_host_vault(root).unwrap();
        assert!(matches!(
            AgentVaultGrantStore::provision_new(&path),
            Err(AgentGrantError::StoreUnavailable)
        ));
        drop(grants);
        let direct = Connection::open(&path).unwrap();
        direct.execute_batch("DROP TABLE agent_vault_audit;").unwrap();
        drop(direct);
        assert!(matches!(
            AgentVaultGrantStore::open_existing(&path),
            Err(AgentGrantError::CorruptStore)
        ));
    }
}
