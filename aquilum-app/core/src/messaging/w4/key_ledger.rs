//! DeviceId-to-WOW W4 Ed25519 pinned key registry, feature-off by default.
//!
//! The registry ONLY accepts an already-authorized host enrollment/revocation.
//! Such claims are sealed types: ordinary network callers cannot construct
//! them, and no production constructor is provided before Akula's authorized
//! invitation and OS-backed identity source is integrated.
//!
//! This SQLite file MUST be kept in guarded Akula private app data and must
//! not replace Akula user identity, File Gate rights or Sync Core. A same-user
//! attacker with DB filesystem write access can still corrupt/replace a
//! database; do NOT treat this as a tamper-proof root or revocation fence.

use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::TrustedRecipientKeys;
use crate::sync::DeviceId;

/// Proof produced by a future authorized Akula host-only invitation flow.
/// Fields are private; no public constructor, no trust-on-first-use.
#[derive(Clone, Copy, Debug)]
pub struct ApprovedEnrollment {
    device: DeviceId,
    epoch: u64,
    ed25519: [u8; 32],
    evidence_digest: [u8; 32],
}

/// A privileged revocation decision issued by the same host authority.
#[derive(Clone, Copy, Debug)]
pub struct ApprovedRevocation {
    device: DeviceId,
    epoch: u64,
    evidence_digest: [u8; 32],
}

#[derive(Debug)]
pub enum LedgerError {
    Database(rusqlite::Error),
    Invalid,
    Conflict,
    StaleEpoch,
    Revoked,
    Missing,
}
impl From<rusqlite::Error> for LedgerError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Database(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedgerOutcome {
    Enrolled,
    Duplicate,
    Rotated,
    Revoked,
}

pub struct W4TrustLedger {
    db: Connection,
}
impl W4TrustLedger {
    /// Caller MUST use a host-validated private application-data location.
    pub fn open(path: &Path) -> Result<Self, LedgerError> {
        let db = Connection::open(path)?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS messaging_w4_trusted_devices(
                device BLOB PRIMARY KEY CHECK(length(device)=16),
                epoch INTEGER NOT NULL CHECK(epoch>0),
                ed25519 BLOB NOT NULL CHECK(length(ed25519)=32),
                evidence BLOB NOT NULL CHECK(length(evidence)=32),
                revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN(0,1))
             );",
        )?;
        Ok(Self { db })
    }

    /// Only trusted host code holding a sealed ApprovedEnrollment may call.
    /// Higher epochs require NEW independently authenticated approval.
    pub fn enroll(&mut self, approved: &ApprovedEnrollment) -> Result<LedgerOutcome, LedgerError> {
        if approved.epoch == 0
            || approved.epoch > i64::MAX as u64
            || approved.ed25519 == [0; 32]
            || approved.evidence_digest == [0; 32]
        {
            return Err(LedgerError::Invalid);
        }
        let device = approved.device.as_uuid().into_bytes();
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<(i64, Vec<u8>, Vec<u8>, i64)> = tx
            .query_row(
                "SELECT epoch,ed25519,evidence,revoked
                 FROM messaging_w4_trusted_devices WHERE device=?1",
                params![device.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let result = match old {
            None => {
                tx.execute(
                    "INSERT INTO messaging_w4_trusted_devices
                        (device,epoch,ed25519,evidence,revoked)
                     VALUES(?1,?2,?3,?4,0)",
                    params![
                        device.as_slice(),
                        approved.epoch as i64,
                        approved.ed25519.as_slice(),
                        approved.evidence_digest.as_slice()
                    ],
                )?;
                LedgerOutcome::Enrolled
            }
            Some((epoch, old_key, old_evidence, revoked)) => {
                if epoch <= 0 {
                    return Err(LedgerError::Conflict);
                }
                if approved.epoch < epoch as u64 {
                    return Err(LedgerError::StaleEpoch);
                }
                if approved.epoch == epoch as u64 {
                    if revoked != 0 {
                        return Err(LedgerError::Revoked);
                    }
                    if old_key.as_slice() != approved.ed25519.as_slice()
                        || old_evidence.as_slice() != approved.evidence_digest.as_slice()
                    {
                        return Err(LedgerError::Conflict);
                    }
                    LedgerOutcome::Duplicate
                } else {
                    tx.execute(
                        "UPDATE messaging_w4_trusted_devices
                         SET epoch=?2,ed25519=?3,evidence=?4,revoked=0
                         WHERE device=?1",
                        params![
                            device.as_slice(),
                            approved.epoch as i64,
                            approved.ed25519.as_slice(),
                            approved.evidence_digest.as_slice()
                        ],
                    )?;
                    LedgerOutcome::Rotated
                }
            }
        };
        tx.commit()?;
        Ok(result)
    }

    /// Revocation is sticky for its current epoch; re-enrollment requires
    /// a higher epoch AND independently confirmed new host approval.
    pub fn revoke(&mut self, approved: &ApprovedRevocation) -> Result<LedgerOutcome, LedgerError> {
        if approved.epoch == 0
            || approved.epoch > i64::MAX as u64
            || approved.evidence_digest == [0; 32]
        {
            return Err(LedgerError::Invalid);
        }
        let device = approved.device.as_uuid().into_bytes();
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<(i64, i64)> = tx
            .query_row(
                "SELECT epoch,revoked FROM messaging_w4_trusted_devices WHERE device=?1",
                params![device.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (epoch, revoked) = old.ok_or(LedgerError::Missing)?;
        if epoch <= 0 || approved.epoch != epoch as u64 {
            return Err(LedgerError::StaleEpoch);
        }
        if revoked != 0 {
            tx.commit()?;
            return Ok(LedgerOutcome::Duplicate);
        }
        tx.execute(
            "UPDATE messaging_w4_trusted_devices
             SET revoked=1,evidence=?3 WHERE device=?1 AND epoch=?2",
            params![
                device.as_slice(),
                approved.epoch as i64,
                approved.evidence_digest.as_slice()
            ],
        )?;
        tx.commit()?;
        Ok(LedgerOutcome::Revoked)
    }
}
impl TrustedRecipientKeys for W4TrustLedger {
    /// Corrupt, unknown, stale or revoked keys fail closed.
    fn key_for(&self, recipient: DeviceId, epoch: u64) -> Option<[u8; 32]> {
        if epoch == 0 || epoch > i64::MAX as u64 {
            return None;
        }
        let device = recipient.as_uuid().into_bytes();
        let row: Option<(i64, Vec<u8>, i64)> = self
            .db
            .query_row(
                "SELECT epoch,ed25519,revoked
                 FROM messaging_w4_trusted_devices WHERE device=?1",
                params![device.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .ok()
            .flatten();
        let (found_epoch, key, revoked) = row?;
        if found_epoch != epoch as i64 || revoked != 0 {
            return None;
        }
        key.as_slice().try_into().ok()
    }
}

#[cfg(test)]
mod tests;
