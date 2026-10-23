//! Durable, issued-first anti-replay gate for one authenticated Iroh read.
//! It cannot mint a grant or authorize a file access. It only consumes a
//! preissued challenge under the SAME currently protected owner policy.

use super::trust::{AdmittedRead, RootTrust};
use super::NodeV1Error;
use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::{params, Connection, Transaction, TransactionBehavior};
use rustix::fs::{open, openat, Mode, OFlags};
use std::fs::{self, File};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const FILE_NAME: &str = "node-challenges.sqlite";
const MAX_PENDING: i64 = 64;
const LIFE_SECONDS: i64 = 60;

pub struct DurableChallengeFence {
    db: Connection,
    trust: RootTrust,
}

fn unix_now() -> Result<i64, NodeV1Error> {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|_| NodeV1Error::Hold("invalid Node system clock"))?
        .as_secs();
    i64::try_from(seconds).map_err(|_| NodeV1Error::Hold("invalid Node clock range"))
}

fn guarded_clock(tx: &Transaction<'_>, now: i64) -> Result<(), NodeV1Error> {
    let last: i64 = tx.query_row(
        "SELECT seen FROM clock_guard WHERE id = 1", [], |row| row.get(0),
    )?;
    if now < last {
        return Err(NodeV1Error::Hold("clock rollback: challenge ledger HOLD"));
    }
    tx.execute("UPDATE clock_guard SET seen = ?1 WHERE id = 1", params![now])?;
    Ok(())
}

impl DurableChallengeFence {
    pub fn open(state: &Path, trust: RootTrust) -> Result<Self, NodeV1Error> {
        trust.require_current()?;
        let metadata = fs::symlink_metadata(state)?;
        if !metadata.is_dir()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.permissions().mode() & 0o7777 != 0o700
        {
            return Err(NodeV1Error::Hold("unsafe Node challenge state directory"));
        }
        let dir = File::from(open(
            state, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW
                | OFlags::CLOEXEC, Mode::empty(),
        ).map_err(std::io::Error::from)?);
        let physical = dir.metadata()?;
        if physical.dev() != metadata.dev() || physical.ino() != metadata.ino() {
            return Err(NodeV1Error::Hold("Node challenge directory replaced"));
        }
        let db_file = match openat(
            &dir, FILE_NAME, OFlags::RDONLY | OFlags::NOFOLLOW
                | OFlags::CLOEXEC, Mode::empty(),
        ) {
            Ok(file) => File::from(file),
            Err(rustix::io::Errno::NOENT) => {
                let fresh = File::from(openat(
                    &dir, FILE_NAME, OFlags::WRONLY | OFlags::CREATE
                        | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::RUSR | Mode::WUSR,
                ).map_err(std::io::Error::from)?);
                fresh.sync_all()?;
                dir.sync_all()?;
                drop(fresh);
                File::from(openat(
                    &dir, FILE_NAME, OFlags::RDONLY | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC, Mode::empty(),
                ).map_err(std::io::Error::from)?)
            }
            Err(error) => return Err(NodeV1Error::Io(error.into())),
        };
        let meta = db_file.metadata()?;
        if !meta.is_file() || meta.uid() != rustix::process::geteuid().as_raw()
            || meta.nlink() != 1 || meta.permissions().mode() & 0o7777 != 0o600
        {
            return Err(NodeV1Error::Hold("unsafe Node challenge database"));
        }
        // The service StateDirectory is 0700 and exclusively owned by its
        // unprivileged DynamicUser. SQLite does not accept an opened dirfd.
        // This does not claim hostile same-UID resistance.
        let db = Connection::open(state.join(FILE_NAME))?;
        db.busy_timeout(Duration::from_secs(3))?;
        db.execute_batch(
            "PRAGMA journal_mode = DELETE;
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;
             PRAGMA secure_delete = ON;",
        )?;
        let version: i64 = db.query_row(
            "PRAGMA user_version", [], |row| row.get(0),
        )?;
        match version {
            0 => db.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE issued (
                   challenge BLOB PRIMARY KEY CHECK(length(challenge) = 32),
                   peer BLOB NOT NULL CHECK(length(peer) = 32),
                   grant_digest BLOB NOT NULL CHECK(length(grant_digest) = 32),
                   policy_digest BLOB NOT NULL CHECK(length(policy_digest) = 32),
                   expires_at INTEGER NOT NULL CHECK(expires_at > 0)
                 );
                 CREATE TABLE clock_guard (
                   id INTEGER PRIMARY KEY CHECK(id = 1),
                   seen INTEGER NOT NULL CHECK(seen >= 0)
                 );
                 INSERT INTO clock_guard(id, seen) VALUES (1, 0);
                 PRAGMA user_version = 1;
                 COMMIT;",
            )?,
            1 => {},
            _ => return Err(NodeV1Error::Hold("unsupported Node challenge schema")),
        }
        let count: i64 = db.query_row(
            "SELECT COUNT(*) FROM clock_guard WHERE id = 1", [], |row| row.get(0),
        )?;
        if count != 1 {
            return Err(NodeV1Error::Hold("corrupt Node challenge clock"));
        }
        Ok(Self { db, trust })
    }

    /// Issue only after RootTrust::admit_read has authenticated the CURRENT
    /// owner-signed roster, exact grant and live QUIC peer identity.
    pub fn issue(&mut self, admission: &AdmittedRead) -> Result<[u8; 32], NodeV1Error> {
        self.trust.require_current()?;
        if admission.policy_digest() != self.trust.policy_digest() {
            return Err(NodeV1Error::Hold("owner policy changed before challenge"));
        }
        let now = unix_now()?;
        let expires = now.checked_add(LIFE_SECONDS)
            .ok_or(NodeV1Error::Hold("invalid challenge expiry"))?;
        let mut nonce = [0u8; 32];
        SystemRandom::new().fill(&mut nonce)
            .map_err(|_| NodeV1Error::Hold("challenge entropy unavailable"))?;
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        guarded_clock(&tx, now)?;
        tx.execute("DELETE FROM issued WHERE expires_at < ?1", params![now])?;
        let pending: i64 = tx.query_row(
            "SELECT COUNT(*) FROM issued", [], |row| row.get(0),
        )?;
        if pending >= MAX_PENDING {
            return Err(NodeV1Error::Hold("Node challenge quota exhausted"));
        }
        tx.execute(
            "INSERT INTO issued(challenge,peer,grant_digest,policy_digest,expires_at)
             VALUES (?1,?2,?3,?4,?5)",
            params![
                nonce.as_slice(), admission.peer().as_slice(),
                admission.grant_digest().as_slice(),
                admission.policy_digest().as_slice(), expires
            ],
        )?;
        tx.commit()?;
        Ok(nonce)
    }

    /// Atomically consumes exactly one previously issued tuple. Absent,
    /// expired, wrong peer/policy or repeated challenges never grant access.
    pub fn consume_once(
        &mut self, peer: [u8; 32], grant_digest: [u8; 32], nonce: [u8; 32],
    ) -> Result<bool, NodeV1Error> {
        self.trust.require_current()?;
        if nonce == [0; 32] || peer == [0; 32] {
            return Ok(false);
        }
        let now = unix_now()?;
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        guarded_clock(&tx, now)?;
        let removed = tx.execute(
            "DELETE FROM issued WHERE challenge = ?1 AND peer = ?2
             AND grant_digest = ?3 AND policy_digest = ?4 AND expires_at >= ?5",
            params![
                nonce.as_slice(), peer.as_slice(), grant_digest.as_slice(),
                self.trust.policy_digest().as_slice(), now
            ],
        )?;
        tx.commit()?;
        Ok(removed == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::trust::{admitted_fixture, policy_fixture};

    #[test]
    fn durable_once_and_wrong_peer_survive_restart() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let trust = policy_fixture(&temp.path().join("root"), [5; 32]);
        let admission = admitted_fixture(&trust);
        let mut ledger = DurableChallengeFence::open(&state, trust.clone()).unwrap();
        let nonce = ledger.issue(&admission).unwrap();
        assert!(!ledger.consume_once([8; 32], [31; 32], nonce).unwrap());
        drop(ledger);
        let mut reopened = DurableChallengeFence::open(&state, trust).unwrap();
        assert!(reopened.consume_once([7; 32], [31; 32], nonce).unwrap());
        assert!(!reopened.consume_once([7; 32], [31; 32], nonce).unwrap());
        assert!(!reopened.consume_once([7; 32], [32; 32], nonce).unwrap());
        assert!(!reopened.consume_once([7; 32], [31; 32], [0; 32]).unwrap());
    }

    #[test]
    fn root_policy_revoke_and_bad_state_permissions_halt() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let owner_dir = temp.path().join("root");
        let trust = policy_fixture(&owner_dir, [6; 32]);
        let admission = admitted_fixture(&trust);
        let mut ledger = DurableChallengeFence::open(&state, trust).unwrap();
        let nonce = ledger.issue(&admission).unwrap();
        fs::write(owner_dir.join("owner-policy.json"), b"revoked").unwrap();
        assert!(ledger.consume_once([7; 32], [31; 32], nonce).is_err());
        assert!(ledger.issue(&admission).is_err());
        fs::set_permissions(&state, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(DurableChallengeFence::open(
            &state, policy_fixture(&temp.path().join("root2"), [6; 32]),
        ).is_err());
    }
}
