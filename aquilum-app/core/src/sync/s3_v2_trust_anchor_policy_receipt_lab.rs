//! E2b: experimental publisher-policy receipt in the EXISTING guarded V2 root row.
//!
//! This adds three nullable columns to s3_v2_trusted_roots, not another DB or
//! independent journal. It is enabled ONLY by signed-vault-policy-receipt-lab.
//! The pre-existing V2 root anchor must already be independently approved.
//! The first policy digest must also be approved through an out-of-S3 channel.
//! SQLite FULL + BEGIN IMMEDIATE protects durable ordinary-crash transitions;
//! a complete malicious rollback of the local DB requires an external witness.
//! There is NO File Gate, S3 write, CAS or Sync permission here.

use super::*;
use super::super::{
    verify_policy_transition, verify_signed_root, ApprovedPublisherKeys,
    ApprovedV2RootScope, DecodedDsseEnvelope, PolicyLabError,
    PublisherPolicyV1, SignedRootError, SignedRootObservation,
};
use super::super::s3_publisher_policy_lab::valid_for_scope;

const RECEIPT_LABEL: &[u8] = b"akula-v2-publisher-policy-receipt-lab-v1";
const POLICY_COLUMNS: [&str; 3] = ["policy_json", "policy_blake3", "policy_receipt"];

#[derive(Debug)]
pub enum V2PolicyReceiptError {
    Root(S3V2TrustAnchorError),
    Db(rusqlite::Error),
    NoAnchoredRoot,
    MissingPolicy,
    InvalidExternalApproval,
    InvalidPolicy,
    CorruptedReceipt,
    ConflictingEpochOrFork,
    NotCurrentlyAnchoredRoot,
    InvalidTransition(PolicyLabError),
    InvalidPublisherSignature(SignedRootError),
}

impl From<S3V2TrustAnchorError> for V2PolicyReceiptError {
    fn from(v: S3V2TrustAnchorError) -> Self { Self::Root(v) }
}
impl From<rusqlite::Error> for V2PolicyReceiptError {
    fn from(v: rusqlite::Error) -> Self { Self::Db(v) }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyReceiptOutcome {
    Inserted,
    Advanced,
    Unchanged,
}

#[derive(Clone, Debug)]
pub struct V2TrustedPublisherPolicy {
    policy: PublisherPolicyV1,
    digest: String,
    receipt: String,
}
impl V2TrustedPublisherPolicy {
    pub fn policy(&self) -> &PublisherPolicyV1 { &self.policy }
    pub fn digest(&self) -> &str { &self.digest }
}

fn private_receipt(
    scope: &S3RemoteAnchorScope,
    canonical_bytes: &[u8],
    policy_blake3: &str,
) -> String {
    let mut digest = blake3::Hasher::new();
    digest.update(RECEIPT_LABEL);
    let vault = scope.keyspace().vault_id().to_string();
    let head = scope.keyspace().current_head_key();
    for part in [
        scope.profile_id().as_bytes(),
        scope.binding_fingerprint().as_bytes(),
        vault.as_bytes(),
        head.as_bytes(),
        scope.policy_bucket().as_bytes(),
        canonical_bytes,
        policy_blake3.as_bytes(),
    ] {
        digest.update(&(part.len() as u64).to_le_bytes());
        digest.update(part);
    }
    digest.finalize().to_hex().to_string()
}

/// Only one row is authoritative; never create a sibling table whose policy
/// generation can diverge from the V2 root's registered profile/scope.
pub(super) fn ensure_policy_columns(db: &Connection) -> Result<(), S3V2TrustAnchorError> {
    let mut statement = db.prepare("PRAGMA table_info(s3_v2_trusted_roots)")?;
    let rows = statement.query_map([], |r| r.get::<_, String>(1))?;
    let names: Vec<String> = rows.collect::<Result<_, _>>()?;
    let matched = POLICY_COLUMNS.iter().filter(|n| names.iter().any(|name| name == **n)).count();
    match matched {
        3 => Ok(()),
        0 => {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE s3_v2_trusted_roots ADD COLUMN policy_json BLOB;
                 ALTER TABLE s3_v2_trusted_roots ADD COLUMN policy_blake3 TEXT;
                 ALTER TABLE s3_v2_trusted_roots ADD COLUMN policy_receipt TEXT;
                 COMMIT;",
            )?;
            Ok(())
        }
        _ => Err(S3V2TrustAnchorError::CorruptedAnchor),
    }
}

fn canonical_policy(
    json: &[u8], scope: &S3RemoteAnchorScope,
) -> Result<(PublisherPolicyV1, Vec<u8>, String), V2PolicyReceiptError> {
    let policy = PublisherPolicyV1::parse_for_review(json)
        .map_err(|_| V2PolicyReceiptError::InvalidPolicy)?;
    if !valid_for_scope(&policy, scope.policy_bucket(), scope.keyspace()) {
        return Err(V2PolicyReceiptError::InvalidPolicy);
    }
    let canonical = serde_json::to_vec(&policy)
        .map_err(|_| V2PolicyReceiptError::InvalidPolicy)?;
    if json != canonical {
        return Err(V2PolicyReceiptError::InvalidPolicy);
    }
    let digest = policy.canonical_digest()
        .map_err(|_| V2PolicyReceiptError::InvalidPolicy)?;
    Ok((policy, canonical, digest))
}

// Validate the policy columns WITHOUT calling S3V2TrustedAnchorJournal.
// The parent root journal calls this directly on all ordinary load() paths,
// including callers that never invoke load_publisher_policy(). Keeping these
// helpers separate prevents circular validation and authorization bypass.
fn checked_policy_row(
    db: &Connection,
    scope: &S3RemoteAnchorScope,
) -> Result<Option<V2TrustedPublisherPolicy>, V2PolicyReceiptError> {
    let row = db.query_row(
        "SELECT policy_json, policy_blake3, policy_receipt
           FROM s3_v2_trusted_roots WHERE profile_id=?1",
        params![scope.profile_id()],
        |r| Ok((
            r.get::<_, Option<Vec<u8>>>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
        )),
    ).optional()?;
    let Some((json, digest, receipt)) = row else {
        return Err(V2PolicyReceiptError::NoAnchoredRoot);
    };
    match (json, digest, receipt) {
        (None, None, None) => Ok(None),
        (Some(json), Some(digest), Some(receipt)) => {
            let (policy, bytes, calculated) = canonical_policy(&json, scope)
                .map_err(|_| V2PolicyReceiptError::CorruptedReceipt)?;
            if calculated != digest
                || receipt != private_receipt(scope, &bytes, &calculated)
            {
                return Err(V2PolicyReceiptError::CorruptedReceipt);
            }
            Ok(Some(V2TrustedPublisherPolicy { policy, digest, receipt }))
        }
        _ => Err(V2PolicyReceiptError::CorruptedReceipt),
    }
}

/// Make ALL ordinary V2 root loads fail closed if a persisted publisher policy
/// is partial/corrupt. A missing policy is allowed only for explicit genesis
/// approval, not a grant of publisher authority.
pub(super) fn validate_policy_columns(
    db: &Connection,
    scope: &S3RemoteAnchorScope,
) -> Result<(), S3V2TrustAnchorError> {
    checked_policy_row(db, scope)
        .map_err(|e| match e {
            V2PolicyReceiptError::Db(err) => S3V2TrustAnchorError::Sqlite(err),
            _ => S3V2TrustAnchorError::CorruptedAnchor,
        })?;
    Ok(())
}

fn load_from(
    db: &Connection,
    scope: &S3RemoteAnchorScope,
) -> Result<Option<V2TrustedPublisherPolicy>, V2PolicyReceiptError> {
    // Require the SAME independently pinned, guarded root row before any
    // publisher policy may be observed or mutated.
    if S3V2TrustedAnchorJournal::load_from(db, scope)?.is_none() {
        return Err(V2PolicyReceiptError::NoAnchoredRoot);
    }
    checked_policy_row(db, scope)
}

impl S3V2TrustedAnchorJournal {
    /// Pure private-data read: returns a policy previously admitted to the
    /// *same row* as the independently pinned V2 root. It does not determine
    /// whether the local SQLite file has been maliciously rolled back.
    pub fn load_publisher_policy(
        &self,
        scope: &S3RemoteAnchorScope,
    ) -> Result<Option<V2TrustedPublisherPolicy>, V2PolicyReceiptError> {
        load_from(&self.db, scope)
    }

    /// Enroll ONLY a policy with an independently confirmed exact genesis
    /// BLAKE3. The caller must prove the out-of-band approval separately; a
    /// digest fetched exclusively from an untrusted bucket is NOT approval.
    ///
    /// The V2 root anchor must already be established with its own independent
    /// genesis pin and pinned-read gate. No policy can create a V2 root row.
    pub fn enroll_externally_approved_genesis_policy(
        &mut self,
        scope: &S3RemoteAnchorScope,
        approved_external_policy_blake3: &str,
        canonical_policy_json: &[u8],
    ) -> Result<PolicyReceiptOutcome, V2PolicyReceiptError> {
        let (policy, bytes, digest) = canonical_policy(canonical_policy_json, scope)?;
        if policy.epoch() != 1
            || !is_blake3(approved_external_policy_blake3)
            || digest != approved_external_policy_blake3
        {
            return Err(V2PolicyReceiptError::InvalidExternalApproval);
        }
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(old) = load_from(&tx, scope)? {
            if old.digest == digest {
                tx.commit()?;
                return Ok(PolicyReceiptOutcome::Unchanged);
            }
            return Err(V2PolicyReceiptError::ConflictingEpochOrFork);
        }
        let receipt = private_receipt(scope, &bytes, &digest);
        let changed = tx.execute(
            "UPDATE s3_v2_trusted_roots
                SET policy_json=?2,policy_blake3=?3,policy_receipt=?4
              WHERE profile_id=?1
                AND scope_fingerprint=?5
                AND policy_json IS NULL
                AND policy_blake3 IS NULL
                AND policy_receipt IS NULL",
            params![scope.profile_id(), bytes, digest, receipt,
                scope.binding_fingerprint()],
        )?;
        if changed != 1 { return Err(V2PolicyReceiptError::ConflictingEpochOrFork); }
        tx.commit()?;
        Ok(PolicyReceiptOutcome::Inserted)
    }

    /// Complete one owner-approved N->N+1 transition atomically, using the
    /// stored OLD policy inside a BEGIN IMMEDIATE transaction. Re-checking
    /// the actual stored state prevents a stale in-memory snapshot from
    /// overwriting a more recent epoch or a same-generation fork.
    ///
    /// This only changes policy columns in the V2 anchor row, not the actual
    /// root/ETag, not Sync baselines and not any user Markdown.
    pub fn record_verified_publisher_transition(
        &mut self,
        scope: &S3RemoteAnchorScope,
        signed: &DecodedDsseEnvelope<'_>,
    ) -> Result<PolicyReceiptOutcome, V2PolicyReceiptError> {
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old = load_from(&tx, scope)?.ok_or(V2PolicyReceiptError::MissingPolicy)?;
        let observation = verify_policy_transition(
            old.policy(), scope.policy_bucket(), scope.keyspace(), signed,
        ).map_err(V2PolicyReceiptError::InvalidTransition)?;
        let (candidate, bytes, digest) = canonical_policy(signed.payload, scope)?;
        if candidate.epoch() != observation.epoch || digest != observation.policy_blake3 {
            return Err(V2PolicyReceiptError::InvalidPolicy);
        }
        let receipt = private_receipt(scope, &bytes, &digest);
        let changed = tx.execute(
            "UPDATE s3_v2_trusted_roots
                SET policy_json=?2,policy_blake3=?3,policy_receipt=?4
              WHERE profile_id=?1
                AND scope_fingerprint=?5
                AND policy_blake3=?6
                AND policy_receipt=?7",
            params![scope.profile_id(), bytes, digest, receipt,
                scope.binding_fingerprint(), old.digest, old.receipt],
        )?;
        if changed != 1 { return Err(V2PolicyReceiptError::ConflictingEpochOrFork); }
        tx.commit()?;
        Ok(PolicyReceiptOutcome::Advanced)
    }

    /// Verify a signature on the EXACT root currently trusted by this
    /// existing V2 journal, using ONLY the latest durably stored publisher
    /// keyset. A revoked key can never be rescued from a stale UI cache.
    ///
    /// Snapshot is held in one BEGIN IMMEDIATE transaction so concurrent
    /// policy rotation and this read cannot select different policy epochs.
    /// This does NOT fetch or re-pin S3 bytes; it is a local preview only.
    pub fn inspect_current_root_publisher_signature(
        &mut self,
        scope: &S3RemoteAnchorScope,
        root: &S3PackedHeadV2,
        attestation: &DecodedDsseEnvelope<'_>,
    ) -> Result<SignedRootObservation, V2PolicyReceiptError> {
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let anchored = Self::load_from(&tx, scope)?
            .ok_or(V2PolicyReceiptError::NoAnchoredRoot)?;
        if anchored.root() != root {
            return Err(V2PolicyReceiptError::NotCurrentlyAnchoredRoot);
        }
        let trusted = load_from(&tx, scope)?.ok_or(V2PolicyReceiptError::MissingPolicy)?;
        let (keys, quorum) = trusted.policy().trusted_publishers();
        let result = verify_signed_root(
            attestation, root,
            &ApprovedV2RootScope {
                bucket: scope.policy_bucket(),
                keyspace: scope.keyspace(),
                policy_epoch: trusted.policy().epoch(),
                previous_trusted_root: anchored.previous_root(),
            },
            &ApprovedPublisherKeys { public_keys: keys, quorum },
        ).map_err(V2PolicyReceiptError::InvalidPublisherSignature)?;
        tx.commit()?;
        Ok(result)
    }
}

#[cfg(feature = "signed-vault-atomic-join-lab")]
#[path = "s3_v2_atomic_join_lab.rs"]
mod atomic_join_lab;
#[cfg(feature = "signed-vault-atomic-join-lab")]
pub use atomic_join_lab::{AtomicJoinError, AtomicJoinOutcome};

#[cfg(feature = "signed-vault-signed-successor-lab")]
#[path = "s3_v2_signed_successor_lab.rs"]
mod signed_successor_lab;
#[cfg(feature = "signed-vault-signed-successor-lab")]
pub use signed_successor_lab::{SignedSuccessorError, SignedSuccessorOutcome};

#[cfg(feature = "signed-vault-receiver-session-lab")]
#[path = "s3_receiver_join_session_lab.rs"]
mod receiver_join_session_lab;
#[cfg(feature = "signed-vault-receiver-session-lab")]
pub use receiver_join_session_lab::{ReceiverJoinSession, ReceiverJoinSessionError};

#[cfg(test)]
#[path = "s3_v2_trust_anchor_policy_receipt_lab_tests.rs"]
mod tests;
