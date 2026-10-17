//! Host-local SQLite consent store (PLUGIN-03).
//! This module does not validate publishers, run plugins, open vault files,
//! or provide a network permission sandbox.

use super::permission_broker::{
    BrokerError, Grant, GrantScope, NetworkHost, NoteId, PluginIdentity,
    RelativeVaultPath, VaultId,
};
use super::Capability;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use uuid::Uuid;

const SCHEMA_VERSION: i64 = 1;
const MAX_SELECTED_IDS: usize = 512;

pub(super) struct PermissionGrantStore {
    connection: Connection,
}

impl PermissionGrantStore {
    pub(super) fn open(path: &Path) -> Result<Self, BrokerError> {
        let connection = Connection::open(path).map_err(|_| BrokerError::StoreFailure)?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = FULL;
                 PRAGMA busy_timeout = 2000;",
            )
            .map_err(|_| BrokerError::StoreFailure)?;

        let version = connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .map_err(|_| BrokerError::StoreFailure)?;
        match version {
            0 => {
                // Dedicated app-data database; fail if conflicting tables already
                // exist instead of silently treating another DB as an empty store.
                connection
                    .execute_batch(
                        "BEGIN IMMEDIATE;
                         CREATE TABLE plugin_principals (
                             vault TEXT NOT NULL,
                             plugin_id TEXT NOT NULL,
                             publisher TEXT NOT NULL,
                             digest BLOB NOT NULL CHECK(length(digest)=32),
                             epoch INTEGER NOT NULL DEFAULT 0 CHECK(epoch>=0),
                             PRIMARY KEY(vault,plugin_id,publisher,digest)
                         );
                         CREATE TABLE plugin_grants (
                             vault TEXT NOT NULL,
                             plugin_id TEXT NOT NULL,
                             publisher TEXT NOT NULL,
                             digest BLOB NOT NULL CHECK(length(digest)=32),
                             capability TEXT NOT NULL,
                             scope_kind TEXT NOT NULL,
                             scope_value TEXT NOT NULL,
                             expires_at INTEGER,
                             PRIMARY KEY(vault,plugin_id,publisher,digest,capability,scope_kind,scope_value),
                             FOREIGN KEY(vault,plugin_id,publisher,digest)
                                 REFERENCES plugin_principals(vault,plugin_id,publisher,digest)
                                 ON DELETE CASCADE
                         );
                         CREATE TABLE plugin_audit (
                             id INTEGER PRIMARY KEY AUTOINCREMENT,
                             event TEXT NOT NULL CHECK(event IN ('grant','revoke')),
                             recorded_at INTEGER NOT NULL DEFAULT (unixepoch()),
                             vault TEXT NOT NULL,
                             plugin_id TEXT NOT NULL,
                             publisher TEXT NOT NULL,
                             digest BLOB NOT NULL,
                             capability TEXT,
                             scope_kind TEXT,
                             epoch INTEGER NOT NULL
                         );
                         PRAGMA user_version = 1;
                         COMMIT;",
                    )
                    .map_err(|_| BrokerError::StoreFailure)?;
            }
            SCHEMA_VERSION => {}
            _ => return Err(BrokerError::UnsupportedStore),
        }
        Ok(Self { connection })
    }

    pub(super) fn epoch(
        &self,
        vault: VaultId,
        identity: &PluginIdentity,
    ) -> Result<u64, BrokerError> {
        let epoch: Option<i64> = self
            .connection
            .query_row(
                "SELECT epoch FROM plugin_principals
                 WHERE vault=?1 AND plugin_id=?2 AND publisher=?3 AND digest=?4",
                params![
                    vault.as_uuid().to_string(),
                    identity.plugin_id(),
                    identity.publisher(),
                    identity.artifact_digest().as_slice()
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| BrokerError::StoreFailure)?;
        let epoch = epoch.unwrap_or(0);
        u64::try_from(epoch).map_err(|_| BrokerError::CorruptStore)
    }

    pub(super) fn load_grants(
        &self,
        vault: VaultId,
        identity: &PluginIdentity,
    ) -> Result<(u64, Vec<Grant>), BrokerError> {
        let epoch = self.epoch(vault, identity)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT capability, scope_kind, scope_value, expires_at
                 FROM plugin_grants WHERE vault=?1 AND plugin_id=?2
                     AND publisher=?3 AND digest=?4
                 ORDER BY capability,scope_kind,scope_value",
            )
            .map_err(|_| BrokerError::StoreFailure)?;
        let rows = statement
            .query_map(
                params![
                    vault.as_uuid().to_string(),
                    identity.plugin_id(),
                    identity.publisher(),
                    identity.artifact_digest().as_slice()
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .map_err(|_| BrokerError::StoreFailure)?;
        let mut grants = Vec::new();
        for row in rows {
            let (capability, scope_kind, scope_value, expires_at) =
                row.map_err(|_| BrokerError::CorruptStore)?;
            let capability: Capability =
                serde_json::from_str(&capability).map_err(|_| BrokerError::CorruptStore)?;
            let scope = decode_scope(&scope_kind, &scope_value)?;
            if expires_at.is_some_and(|value| value <= 0) {
                return Err(BrokerError::CorruptStore);
            }
            grants.push(Grant {
                capability,
                scope,
                expires_at,
            });
        }
        Ok((epoch, grants))
    }

    pub(super) fn persist_grant(
        &mut self,
        vault: VaultId,
        identity: &PluginIdentity,
        grant: &Grant,
    ) -> Result<u64, BrokerError> {
        let capability =
            serde_json::to_string(&grant.capability).map_err(|_| BrokerError::CorruptStore)?;
        let (kind, value) = encode_scope(&grant.scope)?;
        let vault = vault.as_uuid().to_string();
        let digest = identity.artifact_digest();
        let transaction = self.connection.transaction().map_err(|_| BrokerError::StoreFailure)?;
        ensure_principal(&transaction, &vault, identity)?;
        let current = query_epoch(&transaction, &vault, identity)?;
        let changed = transaction
            .execute(
                "INSERT INTO plugin_grants
                    (vault,plugin_id,publisher,digest,capability,scope_kind,scope_value,expires_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(vault,plugin_id,publisher,digest,capability,scope_kind,scope_value)
                 DO UPDATE SET expires_at=excluded.expires_at
                    WHERE plugin_grants.expires_at IS NOT excluded.expires_at",
                params![
                    vault, identity.plugin_id(), identity.publisher(), digest.as_slice(),
                    capability, kind, value, grant.expires_at,
                ],
            )
            .map_err(|_| BrokerError::StoreFailure)?;
        let epoch = if changed > 0 {
            let next = current.checked_add(1).ok_or(BrokerError::RevisionExhausted)?;
            update_epoch(&transaction, &vault, identity, next)?;
            transaction
                .execute(
                    "INSERT INTO plugin_audit
                     (event,vault,plugin_id,publisher,digest,capability,scope_kind,epoch)
                     VALUES('grant',?1,?2,?3,?4,?5,?6,?7)",
                    params![
                        vault, identity.plugin_id(), identity.publisher(), digest.as_slice(),
                        capability, kind, next,
                    ],
                )
                .map_err(|_| BrokerError::StoreFailure)?;
            next
        } else {
            current
        };
        transaction.commit().map_err(|_| BrokerError::StoreFailure)?;
        u64::try_from(epoch).map_err(|_| BrokerError::CorruptStore)
    }

    /// Deletion + durable epoch increment + audit are a single transaction.
    pub(super) fn revoke_all(
        &mut self,
        vault: VaultId,
        identity: &PluginIdentity,
    ) -> Result<u64, BrokerError> {
        let vault = vault.as_uuid().to_string();
        let digest = identity.artifact_digest();
        let transaction = self.connection.transaction().map_err(|_| BrokerError::StoreFailure)?;
        ensure_principal(&transaction, &vault, identity)?;
        let current = query_epoch(&transaction, &vault, identity)?;
        let next = current.checked_add(1).ok_or(BrokerError::RevisionExhausted)?;
        transaction
            .execute(
                "DELETE FROM plugin_grants
                 WHERE vault=?1 AND plugin_id=?2 AND publisher=?3 AND digest=?4",
                params![vault, identity.plugin_id(), identity.publisher(), digest.as_slice()],
            )
            .map_err(|_| BrokerError::StoreFailure)?;
        update_epoch(&transaction, &vault, identity, next)?;
        transaction
            .execute(
                "INSERT INTO plugin_audit
                 (event,vault,plugin_id,publisher,digest,capability,scope_kind,epoch)
                 VALUES('revoke',?1,?2,?3,?4,NULL,NULL,?5)",
                params![vault, identity.plugin_id(), identity.publisher(), digest.as_slice(), next],
            )
            .map_err(|_| BrokerError::StoreFailure)?;
        transaction.commit().map_err(|_| BrokerError::StoreFailure)?;
        u64::try_from(next).map_err(|_| BrokerError::CorruptStore)
    }
}

fn ensure_principal(
    connection: &Connection,
    vault: &str,
    identity: &PluginIdentity,
) -> Result<(), BrokerError> {
    let digest = identity.artifact_digest();
    connection
        .execute(
            "INSERT OR IGNORE INTO plugin_principals
                 (vault,plugin_id,publisher,digest,epoch) VALUES(?1,?2,?3,?4,0)",
            params![vault, identity.plugin_id(), identity.publisher(), digest.as_slice()],
        )
        .map_err(|_| BrokerError::StoreFailure)?;
    Ok(())
}

fn query_epoch(
    connection: &Connection,
    vault: &str,
    identity: &PluginIdentity,
) -> Result<i64, BrokerError> {
    let digest = identity.artifact_digest();
    let value: i64 = connection
        .query_row(
            "SELECT epoch FROM plugin_principals
             WHERE vault=?1 AND plugin_id=?2 AND publisher=?3 AND digest=?4",
            params![vault, identity.plugin_id(), identity.publisher(), digest.as_slice()],
            |row| row.get(0),
        )
        .map_err(|_| BrokerError::StoreFailure)?;
    if value < 0 {
        return Err(BrokerError::CorruptStore);
    }
    Ok(value)
}

fn update_epoch(
    connection: &Connection,
    vault: &str,
    identity: &PluginIdentity,
    next: i64,
) -> Result<(), BrokerError> {
    let digest = identity.artifact_digest();
    connection
        .execute(
            "UPDATE plugin_principals SET epoch=?5
             WHERE vault=?1 AND plugin_id=?2 AND publisher=?3 AND digest=?4",
            params![vault, identity.plugin_id(), identity.publisher(), digest.as_slice(), next],
        )
        .map_err(|_| BrokerError::StoreFailure)?;
    Ok(())
}

fn encode_scope(scope: &GrantScope) -> Result<(&'static str, String), BrokerError> {
    Ok(match scope {
        GrantScope::Action => ("action", String::new()),
        GrantScope::CurrentNote => ("current_note", String::new()),
        GrantScope::WholeVault => ("whole_vault", String::new()),
        GrantScope::Folder(path) => ("folder", path.as_str().to_owned()),
        GrantScope::NetworkHost(host) => ("network_host", host.as_str().to_owned()),
        GrantScope::SelectedNotes(notes) => {
            if notes.is_empty() || notes.len() > MAX_SELECTED_IDS {
                return Err(BrokerError::InvalidScope);
            }
            (
                "selected_notes",
                notes.iter().map(|id| id.as_uuid().to_string()).collect::<Vec<_>>().join(","),
            )
        }
    })
}

fn decode_scope(kind: &str, value: &str) -> Result<GrantScope, BrokerError> {
    let scope = match kind {
        "action" if value.is_empty() => GrantScope::Action,
        "current_note" if value.is_empty() => GrantScope::CurrentNote,
        "whole_vault" if value.is_empty() => GrantScope::WholeVault,
        "folder" => GrantScope::Folder(
            RelativeVaultPath::parse_host_path(value).map_err(|_| BrokerError::CorruptStore)?,
        ),
        "network_host" => GrantScope::NetworkHost(
            NetworkHost::parse_declared_host(value).map_err(|_| BrokerError::CorruptStore)?,
        ),
        "selected_notes" => {
            let parts: Vec<&str> = value.split(',').collect();
            if parts.is_empty() || parts.len() > MAX_SELECTED_IDS {
                return Err(BrokerError::CorruptStore);
            }
            let mut parsed = Vec::new();
            for part in parts {
                let id = Uuid::parse_str(part).map_err(|_| BrokerError::CorruptStore)?;
                if id.to_string() != part {
                    return Err(BrokerError::CorruptStore);
                }
                parsed.push(NoteId::from_uuid_for_host(id));
            }
            let scope = GrantScope::selected_notes(parsed).map_err(|_| BrokerError::CorruptStore)?;
            if encode_scope(&scope).map_err(|_| BrokerError::CorruptStore)?.1 != value {
                return Err(BrokerError::CorruptStore);
            }
            scope
        }
        _ => return Err(BrokerError::CorruptStore),
    };
    Ok(scope)
}
