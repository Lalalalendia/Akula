//! E3c: atomic root + owner-policy enrollment in the EXISTING V2 trust row.
//!
//! Only an explicitly signed and independently approved genesis, alongside a
//! complete bounded/pinned V2 read, may enter this laboratory. Transactional
//! insertion makes crash outcomes all-or-nothing; it is NOT an authenticated
//! out-of-band UI, signer key custody, one-time challenge replay ledger,
//! protection from malicious full-database rollback, or any Sync/File Gate grant.

use super::*;
use super::super::super::{
    inspect_pinned_join_receipt, IndependentlyApprovedJoin, PinnedJoinError,
    PublisherPolicyV1,
};

#[derive(Debug)]
pub enum AtomicJoinError {
    UnverifiedJoin(PinnedJoinError),
    InvalidRoot,
    InvalidPolicy,
    ConflictingBinding,
    Root(S3V2TrustAnchorError),
    Db(rusqlite::Error),
}

impl From<S3V2TrustAnchorError> for AtomicJoinError {
    fn from(err: S3V2TrustAnchorError) -> Self { Self::Root(err) }
}
impl From<rusqlite::Error> for AtomicJoinError {
    fn from(err: rusqlite::Error) -> Self { Self::Db(err) }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AtomicJoinOutcome {
    Inserted,
    Unchanged,
}

impl S3V2TrustedAnchorJournal {
    /// Enroll exact gen-1 V2 root and gen-1 owner policy in ONE guarded SQLite
    /// transaction, **only** after the E3a signed out-of-S3 invitation and E3b
    /// complete pinned-read receipt are independently verified.
    ///
    /// PRECONDITION: human-approved owner keys, root and policy pins were
    /// obtained from a separate authenticated channel, not from the S3 server.
    /// This API cannot prove that the caller honored that provenance rule.
    ///
    /// No change to user notes, File Gate or Sync; the root and policy receipt
    /// are private app-data records only. A system administrator who restores
    /// the *whole* local SQLite file can still roll back this local checkpoint
    /// absent a separate independent witness.
    pub fn enroll_signed_pinned_genesis_atomically(
        &mut self,
        scope: &S3RemoteAnchorScope,
        root: &S3PackedHeadV2,
        root_etag: &str,
        completely_read: &S3PinnedV2ReadPreview,
        policy: &PublisherPolicyV1,
        signed_join: &DecodedDsseEnvelope<'_>,
        approved: &IndependentlyApprovedJoin<'_>,
    ) -> Result<AtomicJoinOutcome, AtomicJoinError> {
        let review = inspect_pinned_join_receipt(
            scope, root, policy, approved, signed_join, root_etag,
            completely_read,
        ).map_err(AtomicJoinError::UnverifiedJoin)?;
        let root_json = root.encode(scope.keyspace())
            .map_err(|_| AtomicJoinError::InvalidRoot)?;
        let policy_json = serde_json::to_vec(policy)
            .map_err(|_| AtomicJoinError::InvalidPolicy)?;
        let (_, canonical, policy_digest) = canonical_policy(&policy_json, scope)
            .map_err(|_| AtomicJoinError::InvalidPolicy)?;
        if review.owner_review.root_blake3
            != root.digest(scope.keyspace())
                .map_err(|_| AtomicJoinError::InvalidRoot)?
            || review.owner_review.policy_blake3 != policy_digest
        {
            return Err(AtomicJoinError::InvalidPolicy);
        }
        let root_receipt = receipt_digest(scope, &root_json, root_etag, None);
        let policy_receipt = private_receipt(scope, &canonical, &policy_digest);

        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // The single profile ID cannot ever silently bind to another scope,
        // another root/ETag, different policy or repaired partial policy.
        if let Some(old) = S3V2TrustedAnchorJournal::load_from(&tx, scope)? {
            let old_policy = load_from(&tx, scope)
                .map_err(|_| AtomicJoinError::ConflictingBinding)?;
            if old.root() == root && old.etag() == root_etag
                && old.previous_root().is_none()
                && old_policy.as_ref().is_some_and(|p| p.digest() == policy_digest)
            {
                tx.commit()?;
                return Ok(AtomicJoinOutcome::Unchanged);
            }
            return Err(AtomicJoinError::ConflictingBinding);
        }

        tx.execute(
            "INSERT INTO s3_v2_trusted_roots (
                profile_id,vault_id,scope_fingerprint,head_key,
                head_json,head_etag,previous_head_json,
                receipt_digest,record_schema,
                policy_json,policy_blake3,policy_receipt
            ) VALUES (?1,?2,?3,?4,?5,?6,NULL,?7,1,?8,?9,?10)",
            params![
                scope.profile_id(),
                scope.keyspace().vault_id().to_string(),
                scope.binding_fingerprint(),
                scope.keyspace().current_head_key(),
                root_json, root_etag, root_receipt,
                canonical, policy_digest, policy_receipt,
            ],
        )?;
        tx.commit()?;
        Ok(AtomicJoinOutcome::Inserted)
    }
}
