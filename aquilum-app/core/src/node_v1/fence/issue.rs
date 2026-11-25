//! Issue read and write challenges with the same durable quota and clock fence.
use super::{DurableChallengeFence, LIFE_SECONDS, MAX_PENDING, guarded_clock, unix_now};
use super::super::{NodeV1Error, trust::{AdmittedPut, AdmittedRead}};
use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::{params, TransactionBehavior};

enum Issuance<'a> {
    Read(&'a AdmittedRead),
    Put(&'a AdmittedPut),
}

impl DurableChallengeFence {
    /// Issue only after RootTrust::admit_read has authenticated the CURRENT
    /// owner-signed roster, exact grant and live QUIC peer identity.
    pub fn issue(&mut self, admission: &AdmittedRead) -> Result<[u8; 32], NodeV1Error> {
        self.issue_challenge(Issuance::Read(admission))
    }

    pub fn issue_put(&mut self, admission: &AdmittedPut) -> Result<[u8; 32], NodeV1Error> {
        self.issue_challenge(Issuance::Put(admission))
    }

    fn issue_challenge(&mut self, admission: Issuance<'_>) -> Result<[u8; 32], NodeV1Error> {
        self.trust.require_current()?;
        let (peer, digest, policy, sql, changed, expiry_error, entropy_error) = match admission {
            Issuance::Read(a) => (a.peer(), a.grant_digest(), a.policy_digest(),
                "INSERT INTO issued(challenge,peer,grant_digest,policy_digest,expires_at)
                 VALUES (?1,?2,?3,?4,?5)",
                "owner policy changed before challenge",
                "invalid challenge expiry", "challenge entropy unavailable"),
            Issuance::Put(a) => (a.scope().device_public_key, a.digest(), a.policy_digest(),
                "INSERT INTO put_issued(challenge,peer,grant_digest,policy_digest,expires_at)
                 VALUES (?1,?2,?3,?4,?5)",
                "owner policy changed before write challenge",
                "invalid Node write expiry", "write challenge entropy unavailable"),
        };
        if policy != self.trust.policy_digest() {
            return Err(NodeV1Error::Hold(changed));
        }
        let now = unix_now()?;
        let expires = now.checked_add(LIFE_SECONDS)
            .ok_or(NodeV1Error::Hold(expiry_error))?;
        let mut nonce = [0u8; 32];
        SystemRandom::new().fill(&mut nonce)
            .map_err(|_| NodeV1Error::Hold(entropy_error))?;
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
        tx.execute(sql, params![nonce.as_slice(), peer.as_slice(),
            digest.as_slice(), policy.as_slice(), expires])?;
        tx.commit()?;
        Ok(nonce)
    }
}
