//! The write path has a separate SQLite table from read challenges. A read
//! capability can never be consumed as a write capability or vice versa.

use super::{
    DurableChallengeFence, guarded_clock, unix_now,
};
use super::super::{NodeV1Error, trust::AdmittedPut};
use rusqlite::{params, TransactionBehavior};

impl DurableChallengeFence {
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
