//! AKAI-AUTH-01a: durable, host-issued MCP agent credentials.
//!
//! This is an inert Core building block, NOT a live authorization gate.
//! No MCP transport calls it yet; an authenticated principal is not a grant.
//! Only trusted host/UI code may issue, rotate or revoke credentials.
//! In particular, MCP clientInfo and agent-declared names are never identity.
//!
//! Store in a dedicated, host-private app-data SQLite file; never in a vault,
//! Markdown or synchronized content. Callers must protect the directory with
//! OS account permissions. A compromised same-user process remains in scope
//! for later OS credential-store integration and is not solved here.

#![allow(dead_code)] // No production caller until the fail-closed 01b router lands.

use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::fmt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

const SCHEMA_VERSION: i64 = 1;
const TOKEN_PREFIX: &str = "akmcp1";
const SECRET_BYTES: usize = 32;
const MAX_LABEL_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) struct AgentPrincipalId(Uuid);

impl AgentPrincipalId {
    pub(crate) fn as_uuid(self) -> Uuid {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) struct AgentCredentialId(Uuid);

impl AgentCredentialId {
    pub(crate) fn as_uuid(self) -> Uuid {
        self.0
    }
}

/// Identity only. This is NOT an authorization to read, search or write a vault.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AuthenticatedAgent {
    pub(crate) principal: AgentPrincipalId,
    /// A later router must revalidate the epoch at protected-use boundaries.
    pub(crate) epoch: u64,
}

/// The complete bearer exists only in this one-time host return value.
/// Intentionally no Debug, Clone, Serialize or Display implementation.
pub(crate) struct IssuedAgentCredential {
    pub(crate) principal: AgentPrincipalId,
    pub(crate) credential: AgentCredentialId,
    pub(crate) epoch: u64,
    bearer: String,
}

impl IssuedAgentCredential {
    /// Move the credential to a zeroizing holder for one-time host display.
    /// A second extraction returns None, even before this struct is dropped.
    pub(crate) fn take_bearer_once(&mut self) -> Option<Zeroizing<String>> {
        if self.bearer.is_empty() {
            return None;
        }
        Some(Zeroizing::new(std::mem::take(&mut self.bearer)))
    }
}

impl Drop for IssuedAgentCredential {
    fn drop(&mut self) {
        self.bearer.zeroize();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentCredentialError {
    Denied,
    InvalidLabel,
    InvalidExpiry,
    EntropyUnavailable,
    StoreUnavailable,
    CorruptStore,
    UnsupportedSchema,
    EpochExhausted,
}

impl fmt::Display for AgentCredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // No credential IDs, secrets, or vault paths are ever included.
        write!(f, "MCP agent credential operation failed: {self:?}")
    }
}

impl std::error::Error for AgentCredentialError {}

pub(crate) struct AgentCredentialStore {
    db: Connection,
}

impl AgentCredentialStore {
    /// The path MUST be provisioned under host-controlled private app-data.
    /// Uses a separate database and never reuses plugin grant tables.
    pub(crate) fn open(path: &Path) -> Result<Self, AgentCredentialError> {
        let db = Connection::open(path).map_err(|_| AgentCredentialError::StoreUnavailable)?;
        db.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA busy_timeout = 2000;",
        )
        .map_err(|_| AgentCredentialError::StoreUnavailable)?;
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|_| AgentCredentialError::StoreUnavailable)?;
        match version {
            0 => {
                // Do not claim an arbitrary existing SQLite file as our own.
                let existing: i64 = db
                    .query_row(
                        "SELECT count(*) FROM sqlite_master
                         WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(|_| AgentCredentialError::StoreUnavailable)?;
                if existing != 0 {
                    return Err(AgentCredentialError::CorruptStore);
                }
                db.execute_batch(
                    "BEGIN IMMEDIATE;
                     CREATE TABLE agent_principals (
                         id TEXT PRIMARY KEY NOT NULL,
                         label TEXT NOT NULL,
                         epoch INTEGER NOT NULL CHECK(epoch >= 0),
                         created_at INTEGER NOT NULL,
                         disabled_at INTEGER
                     );
                     CREATE TABLE agent_credentials (
                         id TEXT PRIMARY KEY NOT NULL,
                         principal TEXT NOT NULL,
                         salt BLOB NOT NULL CHECK(length(salt) = 32),
                         verifier BLOB NOT NULL CHECK(length(verifier) = 32),
                         created_at INTEGER NOT NULL,
                         expires_at INTEGER,
                         revoked_at INTEGER,
                         FOREIGN KEY(principal) REFERENCES agent_principals(id)
                             ON DELETE RESTRICT
                     );
                     CREATE INDEX agent_credentials_by_principal
                         ON agent_credentials(principal);
                     CREATE TABLE agent_credential_audit (
                         sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                         event TEXT NOT NULL CHECK(event IN ('issue','rotate','revoke','disable')),
                         principal TEXT NOT NULL,
                         credential TEXT,
                         epoch INTEGER NOT NULL CHECK(epoch >= 0),
                         recorded_at INTEGER NOT NULL
                     );
                     PRAGMA user_version = 1;
                     COMMIT;",
                )
                .map_err(|_| AgentCredentialError::StoreUnavailable)?;
            }
            SCHEMA_VERSION => {
                // A declared version without its critical schema is corrupt;
                // do not treat a missing table as an empty grant set.
                for name in ["agent_principals", "agent_credentials", "agent_credential_audit"] {
                    let exists: i64 = db
                        .query_row(
                            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                            [name],
                            |row| row.get(0),
                        )
                        .map_err(|_| AgentCredentialError::StoreUnavailable)?;
                    if exists != 1 {
                        return Err(AgentCredentialError::CorruptStore);
                    }
                }
            }
            _ => return Err(AgentCredentialError::UnsupportedSchema),
        }
        Ok(Self { db })
    }

    /// Host creates a new principal with *no* vault grants or tool authority.
    /// Only a salted verifier is persisted. The token cannot be reconstructed.
    pub(crate) fn issue_principal(
        &mut self,
        label: &str,
        expires_at: Option<i64>,
    ) -> Result<IssuedAgentCredential, AgentCredentialError> {
        let label = label.trim();
        if label.is_empty()
            || label.len() > MAX_LABEL_BYTES
            || label.chars().any(|ch| ch.is_control())
        {
            return Err(AgentCredentialError::InvalidLabel);
        }
        let timestamp = now()?;
        validate_expiry(expires_at, timestamp)?;
        let principal = AgentPrincipalId(Uuid::new_v4());
        let pending = mint(principal, 1)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AgentCredentialError::StoreUnavailable)?;
        tx.execute(
            "INSERT INTO agent_principals(id,label,epoch,created_at)
             VALUES(?1,?2,1,?3)",
            params![principal.0.to_string(), label, timestamp],
        )
        .map_err(|_| AgentCredentialError::StoreUnavailable)?;
        insert_credential(&tx, &pending, timestamp, expires_at)?;
        append_audit(&tx, "issue", principal, Some(pending.issued.credential), 1, timestamp)?;
        tx.commit().map_err(|_| AgentCredentialError::StoreUnavailable)?;
        Ok(pending.issued)
    }

    /// Verify a bearer without assigning any permissions. Invalid, missing,
    /// expired and revoked secrets are indistinguishable to the caller.
    pub(crate) fn authenticate(
        &self,
        bearer: &str,
    ) -> Result<AuthenticatedAgent, AgentCredentialError> {
        let (credential, secret) = parse_bearer(bearer)?;
        let record: Option<(String, Vec<u8>, Vec<u8>, Option<i64>, Option<i64>, i64, Option<i64>)> =
            self.db
                .query_row(
                    "SELECT c.principal,c.salt,c.verifier,c.expires_at,c.revoked_at,
                            p.epoch,p.disabled_at
                     FROM agent_credentials c
                     JOIN agent_principals p ON p.id=c.principal WHERE c.id=?1",
                    [credential.0.to_string()],
                    |row| {
                        Ok((
                            row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?,
                            row.get(4)?, row.get(5)?, row.get(6)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| AgentCredentialError::StoreUnavailable)?;
        let Some((principal, salt, verifier, expires_at, revoked_at, epoch, disabled_at)) = record else {
            // Unknown credential IDs must never authorize.
            return Err(AgentCredentialError::Denied);
        };
        if salt.len() != SECRET_BYTES || verifier.len() != SECRET_BYTES || epoch < 0 {
            return Err(AgentCredentialError::CorruptStore);
        }
        let is_valid = verify_mac(&salt, credential, &secret, &verifier);
        if !is_valid || revoked_at.is_some() || disabled_at.is_some() {
            return Err(AgentCredentialError::Denied);
        }
        let current = now()?;
        if expires_at.is_some_and(|expiry| expiry <= current) {
            return Err(AgentCredentialError::Denied);
        }
        let principal = AgentPrincipalId(
            Uuid::parse_str(&principal).map_err(|_| AgentCredentialError::CorruptStore)?,
        );
        Ok(AuthenticatedAgent {
            principal,
            epoch: u64::try_from(epoch).map_err(|_| AgentCredentialError::CorruptStore)?,
        })
    }

    /// A trusted host rotates one specific credential in a single transaction.
    /// A successfully acknowledged rotation invalidates the old bearer for all
    /// *new* admissions. In-flight side effects still require the later fence.
    pub(crate) fn rotate(
        &mut self,
        old: AgentCredentialId,
        expires_at: Option<i64>,
    ) -> Result<IssuedAgentCredential, AgentCredentialError> {
        let timestamp = now()?;
        validate_expiry(expires_at, timestamp)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AgentCredentialError::StoreUnavailable)?;
        let existing: Option<(String, Option<i64>)> = tx
            .query_row(
                "SELECT principal,revoked_at FROM agent_credentials WHERE id=?1",
                [old.0.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| AgentCredentialError::StoreUnavailable)?;
        let (principal, revoked) = existing.ok_or(AgentCredentialError::Denied)?;
        if revoked.is_some() {
            return Err(AgentCredentialError::Denied);
        }
        let principal = AgentPrincipalId(
            Uuid::parse_str(&principal).map_err(|_| AgentCredentialError::CorruptStore)?,
        );
        let epoch = bump_epoch(&tx, principal)?;
        let pending = mint(principal, epoch)?;
        let changed = tx
            .execute(
                "UPDATE agent_credentials SET revoked_at=?2
                 WHERE id=?1 AND revoked_at IS NULL",
                params![old.0.to_string(), timestamp],
            )
            .map_err(|_| AgentCredentialError::StoreUnavailable)?;
        if changed != 1 {
            return Err(AgentCredentialError::Denied);
        }
        insert_credential(&tx, &pending, timestamp, expires_at)?;
        append_audit(
            &tx, "rotate", principal, Some(pending.issued.credential), epoch, timestamp,
        )?;
        tx.commit().map_err(|_| AgentCredentialError::StoreUnavailable)?;
        Ok(pending.issued)
    }

    /// Host-only credential revocation with a durable, monotonic policy epoch.
    pub(crate) fn revoke(
        &mut self,
        credential: AgentCredentialId,
    ) -> Result<u64, AgentCredentialError> {
        let timestamp = now()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AgentCredentialError::StoreUnavailable)?;
        let principal: Option<String> = tx
            .query_row(
                "SELECT principal FROM agent_credentials WHERE id=?1 AND revoked_at IS NULL",
                [credential.0.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| AgentCredentialError::StoreUnavailable)?;
        let principal = principal.ok_or(AgentCredentialError::Denied)?;
        let principal = AgentPrincipalId(
            Uuid::parse_str(&principal).map_err(|_| AgentCredentialError::CorruptStore)?,
        );
        let epoch = bump_epoch(&tx, principal)?;
        let changed = tx
            .execute(
                "UPDATE agent_credentials SET revoked_at=?2
                 WHERE id=?1 AND revoked_at IS NULL",
                params![credential.0.to_string(), timestamp],
            )
            .map_err(|_| AgentCredentialError::StoreUnavailable)?;
        if changed != 1 {
            return Err(AgentCredentialError::Denied);
        }
        append_audit(&tx, "revoke", principal, Some(credential), epoch, timestamp)?;
        tx.commit().map_err(|_| AgentCredentialError::StoreUnavailable)?;
        Ok(epoch)
    }

    /// Disable an entire principal and all of its current/future credentials.
    pub(crate) fn disable_principal(
        &mut self,
        principal: AgentPrincipalId,
    ) -> Result<u64, AgentCredentialError> {
        let timestamp = now()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AgentCredentialError::StoreUnavailable)?;
        let epoch = bump_epoch(&tx, principal)?;
        tx.execute(
            "UPDATE agent_principals SET disabled_at=?2 WHERE id=?1 AND disabled_at IS NULL",
            params![principal.0.to_string(), timestamp],
        )
        .map_err(|_| AgentCredentialError::StoreUnavailable)?;
        tx.execute(
            "UPDATE agent_credentials SET revoked_at=?2
             WHERE principal=?1 AND revoked_at IS NULL",
            params![principal.0.to_string(), timestamp],
        )
        .map_err(|_| AgentCredentialError::StoreUnavailable)?;
        append_audit(&tx, "disable", principal, None, epoch, timestamp)?;
        tx.commit().map_err(|_| AgentCredentialError::StoreUnavailable)?;
        Ok(epoch)
    }
}

struct PendingCredential {
    issued: IssuedAgentCredential,
    salt: [u8; SECRET_BYTES],
    verifier: Vec<u8>,
}

fn mint(principal: AgentPrincipalId, epoch: u64) -> Result<PendingCredential, AgentCredentialError> {
    let rng = SystemRandom::new();
    let credential = AgentCredentialId(Uuid::new_v4());
    let mut secret = Zeroizing::new([0u8; SECRET_BYTES]);
    let mut salt = [0u8; SECRET_BYTES];
    rng.fill(&mut *secret)
        .map_err(|_| AgentCredentialError::EntropyUnavailable)?;
    rng.fill(&mut salt)
        .map_err(|_| AgentCredentialError::EntropyUnavailable)?;
    let verifier = sign_mac(&salt, credential, &secret);
    let mut bearer = format!("{TOKEN_PREFIX}.{}.", credential.0);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in secret.iter() {
        bearer.push(char::from(HEX[usize::from(byte >> 4)]));
        bearer.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(PendingCredential {
        issued: IssuedAgentCredential { principal, credential, epoch, bearer },
        salt,
        verifier,
    })
}

fn insert_credential(
    tx: &Transaction<'_>,
    pending: &PendingCredential,
    timestamp: i64,
    expires_at: Option<i64>,
) -> Result<(), AgentCredentialError> {
    tx.execute(
        "INSERT INTO agent_credentials
         (id,principal,salt,verifier,created_at,expires_at,revoked_at)
         VALUES(?1,?2,?3,?4,?5,?6,NULL)",
        params![
            pending.issued.credential.0.to_string(),
            pending.issued.principal.0.to_string(),
            pending.salt.as_slice(),
            pending.verifier.as_slice(),
            timestamp,
            expires_at,
        ],
    )
    .map_err(|_| AgentCredentialError::StoreUnavailable)?;
    Ok(())
}

fn append_audit(
    tx: &Transaction<'_>,
    event: &str,
    principal: AgentPrincipalId,
    credential: Option<AgentCredentialId>,
    epoch: u64,
    timestamp: i64,
) -> Result<(), AgentCredentialError> {
    let epoch = i64::try_from(epoch).map_err(|_| AgentCredentialError::EpochExhausted)?;
    tx.execute(
        "INSERT INTO agent_credential_audit(event,principal,credential,epoch,recorded_at)
         VALUES(?1,?2,?3,?4,?5)",
        params![
            event,
            principal.0.to_string(),
            credential.map(|id| id.0.to_string()),
            epoch,
            timestamp,
        ],
    )
    .map_err(|_| AgentCredentialError::StoreUnavailable)?;
    Ok(())
}

fn bump_epoch(
    tx: &Transaction<'_>,
    principal: AgentPrincipalId,
) -> Result<u64, AgentCredentialError> {
    let current: Option<(i64, Option<i64>)> = tx
        .query_row(
            "SELECT epoch,disabled_at FROM agent_principals WHERE id=?1",
            [principal.0.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| AgentCredentialError::StoreUnavailable)?;
    let (epoch, disabled_at) = current.ok_or(AgentCredentialError::Denied)?;
    if disabled_at.is_some() {
        return Err(AgentCredentialError::Denied);
    }
    let next = epoch
        .checked_add(1)
        .filter(|epoch| *epoch > 0)
        .ok_or(AgentCredentialError::EpochExhausted)?;
    tx.execute(
        "UPDATE agent_principals SET epoch=?2 WHERE id=?1",
        params![principal.0.to_string(), next],
    )
    .map_err(|_| AgentCredentialError::StoreUnavailable)?;
    u64::try_from(next).map_err(|_| AgentCredentialError::CorruptStore)
}

fn sign_mac(salt: &[u8], id: AgentCredentialId, secret: &[u8; SECRET_BYTES]) -> Vec<u8> {
    let key = hmac::Key::new(hmac::HMAC_SHA256, salt);
    let mut message = [0u8; 16 + SECRET_BYTES];
    message[..16].copy_from_slice(id.0.as_bytes());
    message[16..].copy_from_slice(secret);
    let result = hmac::sign(&key, &message).as_ref().to_vec();
    message.zeroize();
    result
}

fn verify_mac(
    salt: &[u8],
    id: AgentCredentialId,
    secret: &[u8; SECRET_BYTES],
    verifier: &[u8],
) -> bool {
    let key = hmac::Key::new(hmac::HMAC_SHA256, salt);
    let mut message = [0u8; 16 + SECRET_BYTES];
    message[..16].copy_from_slice(id.0.as_bytes());
    message[16..].copy_from_slice(secret);
    let valid = hmac::verify(&key, &message, verifier).is_ok();
    message.zeroize();
    valid
}

fn parse_bearer(
    bearer: &str,
) -> Result<(AgentCredentialId, Zeroizing<[u8; SECRET_BYTES]>), AgentCredentialError> {
    let mut parts = bearer.split('.');
    if parts.next() != Some(TOKEN_PREFIX) {
        return Err(AgentCredentialError::Denied);
    }
    let id = parts.next().ok_or(AgentCredentialError::Denied)?;
    let encoded = parts.next().ok_or(AgentCredentialError::Denied)?;
    if parts.next().is_some() || id.len() != 36 || encoded.len() != SECRET_BYTES * 2 {
        return Err(AgentCredentialError::Denied);
    }
    let id = Uuid::parse_str(id).map_err(|_| AgentCredentialError::Denied)?;
    let mut secret = Zeroizing::new([0u8; SECRET_BYTES]);
    for (index, pair) in encoded.as_bytes().chunks_exact(2).enumerate() {
        let hi = nibble(pair[0]).ok_or(AgentCredentialError::Denied)?;
        let lo = nibble(pair[1]).ok_or(AgentCredentialError::Denied)?;
        secret[index] = (hi << 4) | lo;
    }
    Ok((AgentCredentialId(id), secret))
}

fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn validate_expiry(expiry: Option<i64>, current: i64) -> Result<(), AgentCredentialError> {
    if expiry.is_some_and(|expiry| expiry <= current) {
        return Err(AgentCredentialError::InvalidExpiry);
    }
    Ok(())
}

fn now() -> Result<i64, AgentCredentialError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AgentCredentialError::StoreUnavailable)?
        .as_secs();
    i64::try_from(seconds).map_err(|_| AgentCredentialError::StoreUnavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn two_host_issued_agents_are_distinct_and_no_secret_is_saved() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-credentials.sqlite3");
        let mut store = AgentCredentialStore::open(&path).unwrap();
        let mut alice = store.issue_principal("Agent A", None).unwrap();
        let mut bob = store.issue_principal("Agent B", None).unwrap();
        let alice_token = alice.take_bearer_once().unwrap();
        let bob_token = bob.take_bearer_once().unwrap();
        assert!(alice.take_bearer_once().is_none());
        assert!(bob.take_bearer_once().is_none());
        assert_ne!(alice.principal, bob.principal);
        assert_ne!(alice.credential, bob.credential);
        assert_ne!(alice_token.as_str(), bob_token.as_str());
        assert_eq!(store.authenticate(alice_token.as_str()).unwrap().principal, alice.principal);
        assert_eq!(store.authenticate(bob_token.as_str()).unwrap().principal, bob.principal);
        let forged = format!(
            "{TOKEN_PREFIX}.{}.{}",
            bob.credential.0,
            alice_token.split('.').next_back().unwrap()
        );
        assert_eq!(store.authenticate(&forged), Err(AgentCredentialError::Denied));
        let secret = alice_token.as_bytes().to_vec();
        drop(store);
        let disk = std::fs::read(&path).unwrap();
        assert!(!disk.windows(secret.len()).any(|window| window == secret));
    }

    #[test]
    fn revoke_is_durable_and_unknown_bearers_fail_closed() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agents.sqlite3");
        let mut store = AgentCredentialStore::open(&path).unwrap();
        let mut issued = store.issue_principal("Reader", None).unwrap();
        let token = issued.take_bearer_once().unwrap();
        let old_epoch = store.authenticate(&token).unwrap().epoch;
        let new_epoch = store.revoke(issued.credential).unwrap();
        assert!(new_epoch > old_epoch);
        assert_eq!(store.authenticate(&token), Err(AgentCredentialError::Denied));
        assert_eq!(store.authenticate("akmcp1.broken"), Err(AgentCredentialError::Denied));
        assert_eq!(store.revoke(issued.credential), Err(AgentCredentialError::Denied));
        drop(store);
        let reopened = AgentCredentialStore::open(&path).unwrap();
        assert_eq!(reopened.authenticate(&token), Err(AgentCredentialError::Denied));
    }

    #[test]
    fn independent_process_connections_cannot_both_revoke_and_rotate() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let dir = tempdir().unwrap();
        let path = dir.path().join("agents.sqlite3");
        let mut host = AgentCredentialStore::open(&path).unwrap();
        let mut issued = host.issue_principal("Concurrent agent", None).unwrap();
        let old_token = issued.take_bearer_once().unwrap();
        let id = issued.credential;
        drop(host);

        let start = Arc::new(Barrier::new(3));
        let revoke_path = path.clone();
        let revoke_barrier = Arc::clone(&start);
        let revoker = thread::spawn(move || {
            let mut store = AgentCredentialStore::open(&revoke_path).unwrap();
            revoke_barrier.wait();
            store.revoke(id)
        });
        let rotate_path = path.clone();
        let rotate_barrier = Arc::clone(&start);
        let rotator = thread::spawn(move || {
            let mut store = AgentCredentialStore::open(&rotate_path).unwrap();
            rotate_barrier.wait();
            store.rotate(id, None)
        });
        start.wait();
        let revoked = revoker.join().unwrap();
        let rotated = rotator.join().unwrap();
        assert_ne!(revoked.is_ok(), rotated.is_ok(), "exactly one transaction must win");
        assert_eq!(
            revoked.as_ref().err().or(rotated.as_ref().err()),
            Some(&AgentCredentialError::Denied)
        );

        let reopened = AgentCredentialStore::open(&path).unwrap();
        assert_eq!(reopened.authenticate(&old_token), Err(AgentCredentialError::Denied));
        if let Ok(mut new_key) = rotated {
            let new_token = new_key.take_bearer_once().unwrap();
            assert_eq!(
                reopened.authenticate(new_token.as_str()).unwrap().principal,
                new_key.principal
            );
        }
    }

    #[test]
    fn rotate_invalidates_old_bearer_and_increments_epoch_once() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agents.sqlite3");
        let mut store = AgentCredentialStore::open(&path).unwrap();
        let mut first = store.issue_principal("Indexer", None).unwrap();
        let original = first.take_bearer_once().unwrap();
        let mut next = store.rotate(first.credential, None).unwrap();
        let next_token = next.take_bearer_once().unwrap();
        assert_eq!(first.principal, next.principal);
        assert_eq!(next.epoch, first.epoch + 1);
        assert_eq!(store.authenticate(&original), Err(AgentCredentialError::Denied));
        assert_eq!(store.authenticate(next_token.as_str()).unwrap().epoch, next.epoch);
        drop(store);
        let reopened = AgentCredentialStore::open(&path).unwrap();
        assert_eq!(reopened.authenticate(&original), Err(AgentCredentialError::Denied));
        assert!(reopened.authenticate(next_token.as_str()).is_ok());
    }

    #[test]
    fn disabled_principal_stays_denied_after_reopen() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agents.sqlite3");
        let mut store = AgentCredentialStore::open(&path).unwrap();
        let mut issued = store.issue_principal("Job runner", None).unwrap();
        let token = issued.take_bearer_once().unwrap();
        let epoch = store.disable_principal(issued.principal).unwrap();
        assert!(epoch > issued.epoch);
        assert_eq!(store.authenticate(token.as_str()), Err(AgentCredentialError::Denied));
        assert_eq!(store.rotate(issued.credential, None).err(), Some(AgentCredentialError::Denied));
        drop(store);
        let reopened = AgentCredentialStore::open(&path).unwrap();
        assert_eq!(reopened.authenticate(token.as_str()), Err(AgentCredentialError::Denied));
    }

    #[test]
    fn expiry_tamper_and_invalid_labels_fail_closed() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agents.sqlite3");
        let mut store = AgentCredentialStore::open(&path).unwrap();
        assert_eq!(store.issue_principal("", None).err(), Some(AgentCredentialError::InvalidLabel));
        assert_eq!(
            store.issue_principal("agent", Some(1)).err(),
            Some(AgentCredentialError::InvalidExpiry)
        );
        let mut issued = store.issue_principal("Temporary", None).unwrap();
        let token = issued.take_bearer_once().unwrap();
        store.db.execute(
            "UPDATE agent_credentials SET expires_at=1 WHERE id=?1",
            [issued.credential.0.to_string()],
        ).unwrap();
        assert_eq!(store.authenticate(token.as_str()), Err(AgentCredentialError::Denied));
    }

    #[test]
    fn arbitrary_existing_database_and_wrong_schema_are_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("intruder.sqlite3");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE unrelated (id TEXT);").unwrap();
        drop(db);
        assert!(matches!(AgentCredentialStore::open(&path), Err(AgentCredentialError::CorruptStore)));

        let fresh = dir.path().join("unsupported.sqlite3");
        let store = AgentCredentialStore::open(&fresh).unwrap();
        drop(store);
        let db = Connection::open(&fresh).unwrap();
        db.execute_batch("PRAGMA user_version=99;").unwrap();
        drop(db);
        assert!(matches!(
            AgentCredentialStore::open(&fresh),
            Err(AgentCredentialError::UnsupportedSchema)
        ));
    }
}
