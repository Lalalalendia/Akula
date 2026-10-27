//! The write path has a separate SQLite table from read challenges. A read
//! capability can never be consumed as a write capability or vice versa.

use super::{
    DurableChallengeFence, LIFE_SECONDS, MAX_PENDING, guarded_clock, unix_now,
};
use super::super::{NodeV1Error, trust::AdmittedPut};
use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::{params, TransactionBehavior};

impl DurableChallengeFence {
    pub fn issue_put(&mut self, admission: &AdmittedPut)
        -> Result<[u8; 32], NodeV1Error>
    {
        self.trust.require_current()?;
        if admission.policy_digest() != self.trust.policy_digest() {
            return Err(NodeV1Error::Hold("owner policy changed before write challenge"));
        }
        let now = unix_now()?;
        let expiry = now.checked_add(LIFE_SECONDS)
            .ok_or(NodeV1Error::Hold("invalid Node write expiry"))?;
        let mut nonce = [0u8; 32];
        SystemRandom::new().fill(&mut nonce)
            .map_err(|_| NodeV1Error::Hold("write challenge entropy unavailable"))?;
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        guarded_clock(&tx, now)?;
        tx.execute("DELETE FROM issued WHERE expires_at < ?1", params![now])?;
        tx.execute("DELETE FROM put_issued WHERE expires_at < ?1", params![now])?;
        let pending: i64 = tx.query_row(
            "SELECT (SELECT COUNT(*) FROM issued) + (SELECT COUNT(*) FROM put_issued)",
            [], |row| row.get(0),
        )?;
        if pending >= MAX_PENDING {
            return Err(NodeV1Error::Hold("Node challenge quota exhausted"));
        }
        tx.execute(
            "INSERT INTO put_issued(challenge,peer,grant_digest,policy_digest,expires_at)
             VALUES (?1,?2,?3,?4,?5)",
            params![
                nonce.as_slice(), admission.scope().device_public_key.as_slice(),
                admission.digest().as_slice(), admission.policy_digest().as_slice(),
                expiry,
            ],
        )?;
        tx.commit()?;
        Ok(nonce)
    }

    pub fn consume_put(
        &mut self, admission: &AdmittedPut, nonce: [u8; 32],
    ) -> Result<bool, NodeV1Error> {
        self.trust.require_current()?;
        if admission.policy_digest() != self.trust.policy_digest()
            || nonce == [0; 32]
        {
            return Ok(false);
        }
        let now = unix_now()?;
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        guarded_clock(&tx, now)?;
        let removed = tx.execute(
            "DELETE FROM put_issued
             WHERE challenge = ?1 AND peer = ?2 AND grant_digest = ?3
               AND policy_digest = ?4 AND expires_at >= ?5",
            params![
                nonce.as_slice(), admission.scope().device_public_key.as_slice(),
                admission.digest().as_slice(), admission.policy_digest().as_slice(),
                now,
            ],
        )?;
        tx.commit()?;
        Ok(removed == 1)
    }
}
