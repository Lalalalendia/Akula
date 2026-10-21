//! E3d (research only): a separately signed direct V2 successor admission.
//!
//! Unlike the legacy root-only API, this path requires a complete pinned
//! read AND an Ed25519 quorum from the CURRENT guarded publisher policy.
//! No S3 PUT, File Gate, local note changes, automatic Sync, or writer token.

use super::*;

#[derive(Debug)]
pub enum SignedSuccessorError {
    Root(S3V2TrustAnchorError),
    Policy(V2PolicyReceiptError),
    InvalidPinnedRead,
    InvalidRoot,
    InvalidSignature(SignedRootError),
    Db(rusqlite::Error),
}

impl From<S3V2TrustAnchorError> for SignedSuccessorError {
    fn from(value: S3V2TrustAnchorError) -> Self { Self::Root(value) }
}
impl From<V2PolicyReceiptError> for SignedSuccessorError {
    fn from(value: V2PolicyReceiptError) -> Self { Self::Policy(value) }
}
impl From<rusqlite::Error> for SignedSuccessorError {
    fn from(value: rusqlite::Error) -> Self { Self::Db(value) }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignedSuccessorOutcome {
    Advanced,
    Unchanged,
}

impl S3V2TrustedAnchorJournal {
    /// Advance an already enrolled V2 Vault by exactly one signed generation.
    ///
    /// Preconditions:
    /// - `pinned` attests the exact complete S3 root/ETag/manifest/index/packs
    ///   after the reader's final remote-root consistency check.
    /// - `attestation` contains the *exact* signed bytes of that new root.
    ///
    /// In one BEGIN IMMEDIATE transaction, load and validate the CURRENT
    /// policy, require its distinct authorized publisher quorum, then advance
    /// the existing root row. A parallel revocation/fork cannot be committed
    /// against a cached or caller-supplied policy.
    ///
    /// Signature verification is not proof of global freshness or a license
    /// to download user notes, grant File Gate or write to S3.
    pub fn record_verified_signed_successor(
        &mut self,
        scope: &S3RemoteAnchorScope,
        candidate: &S3PackedHeadV2,
        etag: &str,
        pinned: &S3PinnedV2ReadPreview,
        attestation: &DecodedDsseEnvelope<'_>,
    ) -> Result<SignedSuccessorOutcome, SignedSuccessorError> {
        if candidate.generation() <= 1
            || !valid_etag(etag)
            || !pinned.attests_exact_root(scope, candidate, etag)
        {
            return Err(SignedSuccessorError::InvalidPinnedRead);
        }
        let ns = scope.keyspace();
        let candidate_bytes = candidate.encode(ns)
            .map_err(|_| SignedSuccessorError::InvalidRoot)?;

        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let anchored = S3V2TrustedAnchorJournal::load_from(&tx, scope)?
            .ok_or(S3V2TrustAnchorError::MissingTrustedAnchor)?;
        // No implicit policy enrollment or fallback to an unsigned root-only
        // history. The root and publisher policy are read under the same lock.
        let trusted = load_from(&tx, scope)?
            .ok_or(V2PolicyReceiptError::MissingPolicy)?;

        let same = anchored.root() == candidate;
        if same {
            if anchored.etag() != etag {
                return Err(S3V2TrustAnchorError::RootRewritten.into());
            }
        } else {
            if candidate.generation() <= anchored.root().generation() {
                return Err(S3V2TrustAnchorError::RollbackOrFork.into());
            }
            if anchored.root().generation().checked_add(1) != Some(candidate.generation())
                || !candidate.is_direct_successor_of(ns, anchored.root())
            {
                return Err(S3V2TrustAnchorError::IncompleteAncestry.into());
            }
        }

        let (keys, quorum) = trusted.policy().trusted_publishers();
        verify_signed_root(
            attestation, candidate,
            &ApprovedV2RootScope {
                bucket: scope.policy_bucket(),
                keyspace: ns,
                policy_epoch: trusted.policy().epoch(),
                previous_trusted_root: if same {
                    anchored.previous_root()
                } else {
                    Some(anchored.root())
                },
            },
            &ApprovedPublisherKeys { public_keys: keys, quorum },
        ).map_err(SignedSuccessorError::InvalidSignature)?;

        if same {
            tx.commit()?;
            return Ok(SignedSuccessorOutcome::Unchanged);
        }

        let prior_bytes = anchored.root().encode(ns)
            .map_err(|_| SignedSuccessorError::InvalidRoot)?;
        let prior_prior_bytes = anchored.previous_root()
            .map(|prior| prior.encode(ns)).transpose()
            .map_err(|_| SignedSuccessorError::InvalidRoot)?;
        let expected_receipt = receipt_digest(
            scope, &prior_bytes, anchored.etag(), prior_prior_bytes.as_deref(),
        );
        let new_receipt = receipt_digest(scope, &candidate_bytes, etag, Some(&prior_bytes));

        let changed = tx.execute(
            "UPDATE s3_v2_trusted_roots
                SET head_json=?2, head_etag=?3, previous_head_json=?4,
                    receipt_digest=?5
              WHERE profile_id=?1
                AND scope_fingerprint=?6
                AND receipt_digest=?7
                AND policy_blake3=?8
                AND policy_receipt=?9",
            params![
                scope.profile_id(), candidate_bytes, etag, prior_bytes,
                new_receipt, scope.binding_fingerprint(), expected_receipt,
                trusted.digest(), trusted.receipt.as_str(),
            ],
        )?;
        if changed != 1 {
            return Err(S3V2TrustAnchorError::RollbackOrFork.into());
        }
        tx.commit()?;
        Ok(SignedSuccessorOutcome::Advanced)
    }
}
