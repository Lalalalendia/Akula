//! SYNC-05d2j: crash-durable, explicit V2 S3 remote observation anchors.
//!
//! This is separate from the existing V1 S3 anchor and all per-document
//! confirmed sync baselines. It has no S3 writer, File Gate writer, scheduler,
//! TOFU/cloud-root enrollment or credential persistence. Trust in an initial
//! root MUST originate from an independently authorized root fingerprint.
//! Neither SQLite nor unkeyed BLAKE3 proves a remote writer's identity.
//!
//! Open with Core RegisteredReadAdmission: SQLite and WAL must live in guarded
//! private app-data, never in the user's Markdown workspace.

use super::{
    RegisteredReadAdmission, S3PackedHeadV2, S3PinnedV2ReadError,
    S3PinnedV2ReadPreview, S3RemoteAnchorScope, VaultPortError,
    AwsS3ProbeClient,
};
use super::profile_binding::validate_appdata_location;
use super::s3_manifest::is_blake3;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::path::Path;

const RECORD_SCHEMA: i64 = 1;
const RECORD_LABEL: &[u8] = b"akula-v2-explicit-trusted-root-anchor-v1";

#[derive(Debug)]
pub enum S3V2TrustAnchorError {
    Admission(VaultPortError),
    InvalidStoreLocation,
    Sqlite(rusqlite::Error),
    MissingTrustedAnchor,
    InvalidExternalPin,
    UnverifiedRead,
    ScopeChanged,
    CorruptedAnchor,
    UnsupportedSchema,
    RollbackOrFork,
    RootRewritten,
    IncompleteAncestry,
    Reader(S3PinnedV2ReadError),
}

impl From<VaultPortError> for S3V2TrustAnchorError {
    fn from(e: VaultPortError) -> Self { Self::Admission(e) }
}
impl From<rusqlite::Error> for S3V2TrustAnchorError {
    fn from(e: rusqlite::Error) -> Self { Self::Sqlite(e) }
}
impl From<S3PinnedV2ReadError> for S3V2TrustAnchorError {
    fn from(e: S3PinnedV2ReadError) -> Self { Self::Reader(e) }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum S3V2TrustAnchorOutcome {
    Inserted,
    Advanced,
    Unchanged,
}

/// Read-only checkpoint from the host's explicitly authorized V2 root.
/// It cannot confirm local notes, publish S3, or create File Gate authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3V2TrustedAnchor {
    root: S3PackedHeadV2,
    root_etag: String,
    previous_root: Option<S3PackedHeadV2>,
}

impl S3V2TrustedAnchor {
    pub fn root(&self) -> &S3PackedHeadV2 { &self.root }
    pub fn etag(&self) -> &str { &self.root_etag }
    pub fn previous_root(&self) -> Option<&S3PackedHeadV2> {
        self.previous_root.as_ref()
    }
}

pub struct S3V2TrustedAnchorJournal {
    db: Connection,
}

fn valid_etag(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256
        && !s.bytes().any(|b| b.is_ascii_control())
}

fn receipt_digest(
    scope: &S3RemoteAnchorScope,
    root_bytes: &[u8],
    etag: &str,
    previous_bytes: Option<&[u8]>,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(RECORD_LABEL);
    let previous_flag = if previous_bytes.is_some() { b"1" } else { b"0" };
    let vault = scope.keyspace().vault_id().to_string();
    let head_key = scope.keyspace().current_head_key();
    for part in [
        scope.profile_id().as_bytes(),
        vault.as_bytes(),
        scope.binding_fingerprint().as_bytes(),
        head_key.as_bytes(),
        root_bytes,
        etag.as_bytes(),
        previous_flag,
        previous_bytes.unwrap_or_default(),
    ] {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    hasher.finalize().to_hex().to_string()
}

impl S3V2TrustedAnchorJournal {
    pub fn open(
        admission: &RegisteredReadAdmission<'_>,
        private_db: &Path,
    ) -> Result<Self, S3V2TrustAnchorError> {
        admission.recheck_registered_root()?;
        validate_appdata_location(admission, private_db)
            .map_err(|_| S3V2TrustAnchorError::InvalidStoreLocation)?;
        let db = Connection::open(private_db)?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;
             CREATE TABLE IF NOT EXISTS s3_v2_trusted_roots (
                 profile_id TEXT PRIMARY KEY NOT NULL,
                 vault_id TEXT NOT NULL,
                 scope_fingerprint TEXT NOT NULL,
                 head_key TEXT NOT NULL,
                 head_json BLOB NOT NULL,
                 head_etag TEXT NOT NULL,
                 previous_head_json BLOB,
                 receipt_digest TEXT NOT NULL,
                 record_schema INTEGER NOT NULL CHECK(record_schema=1)
             );",
        )?;
        // E2b is an inert, opt-in extension of THIS existing V2 trust row.
        // Production Sync / normal installed Akula never run this migration.
        #[cfg(feature = "signed-vault-policy-receipt-lab")]
        policy_receipt_lab::ensure_policy_columns(&db)?;
        admission.recheck_registered_root()?;
        Ok(Self { db })
    }

    /// A missing persisted root is NOT an empty/default S3 vault. A profile
    /// ID reused for another account/bucket/vault fails rather than accepting
    /// the new scope, even if the same credentials happen to read that S3.
    pub fn load(
        &self, scope: &S3RemoteAnchorScope,
    ) -> Result<Option<S3V2TrustedAnchor>, S3V2TrustAnchorError> {
        Self::load_from(&self.db, scope)
    }

    fn load_from(
        db: &Connection, scope: &S3RemoteAnchorScope,
    ) -> Result<Option<S3V2TrustedAnchor>, S3V2TrustAnchorError> {
        let row = db.query_row(
            "SELECT vault_id,scope_fingerprint,head_key,head_json,
                    head_etag,previous_head_json,receipt_digest,record_schema
               FROM s3_v2_trusted_roots WHERE profile_id=?1",
            params![scope.profile_id()],
            |r| Ok((
                r.get::<_, String>(0)?, r.get::<_, String>(1)?,
                r.get::<_, String>(2)?, r.get::<_, Vec<u8>>(3)?,
                r.get::<_, String>(4)?, r.get::<_, Option<Vec<u8>>>(5)?,
                r.get::<_, String>(6)?, r.get::<_, i64>(7)?,
            )),
        ).optional()?;
        let Some((vault, fp, head_key, root_json, etag, previous_json,
            receipt, schema)) = row else {
            return Ok(None);
        };
        if schema != RECORD_SCHEMA {
            return Err(S3V2TrustAnchorError::UnsupportedSchema);
        }
        if vault != scope.keyspace().vault_id().to_string()
            || fp != scope.binding_fingerprint()
            || head_key != scope.keyspace().current_head_key()
        {
            return Err(S3V2TrustAnchorError::ScopeChanged);
        }
        if !valid_etag(&etag) || receipt != receipt_digest(
            scope, &root_json, &etag, previous_json.as_deref(),
        ) {
            return Err(S3V2TrustAnchorError::CorruptedAnchor);
        }
        let ns = scope.keyspace();
        let root = S3PackedHeadV2::decode(ns, &root_json)
            .map_err(|_| S3V2TrustAnchorError::CorruptedAnchor)?;
        let previous = previous_json.as_ref().map(|json| {
            S3PackedHeadV2::decode(ns, json)
                .map_err(|_| S3V2TrustAnchorError::CorruptedAnchor)
        }).transpose()?;
        if root.encode(ns).ok().as_deref() != Some(root_json.as_slice())
            || previous.as_ref().and_then(|head| head.encode(ns).ok())
                .as_deref() != previous_json.as_deref()
            || (root.generation() == 1 && previous.is_some())
            || (root.generation() > 1
                && !previous.as_ref().is_some_and(|old|
                    root.is_direct_successor_of(ns, old)))
        {
            return Err(S3V2TrustAnchorError::CorruptedAnchor);
        }
        // In the opt-in E2b lab, even callers using the OLD root-only API
        // must fail closed if the associated publisher-policy receipt is
        // partially written or corrupted. This is NOT a second trust journal.
        #[cfg(feature = "signed-vault-policy-receipt-lab")]
        policy_receipt_lab::validate_policy_columns(db, scope)?;
        Ok(Some(S3V2TrustedAnchor {
            root, root_etag: etag, previous_root: previous,
        }))
    }

    /// Enroll ONLY a newly created V2 genesis whose complete remote bytes
    /// were already fully inspected, and whose BLAKE3 was independently
    /// authorized out-of-band. An unknown server-supplied digest alone is
    /// never sufficient evidence of the user's intended publisher.
    ///
    /// No V1-to-V2 migration and no adoption of an arbitrary existing V2
    /// generation is allowed. A rebinding of an existing profile is refused.
    pub fn enroll_verified_genesis(
        &mut self,
        scope: &S3RemoteAnchorScope,
        approved_external_root_blake3: &str,
        root: &S3PackedHeadV2,
        etag: &str,
        verified: &S3PinnedV2ReadPreview,
    ) -> Result<S3V2TrustAnchorOutcome, S3V2TrustAnchorError> {
        if !is_blake3(approved_external_root_blake3)
            || root.generation() != 1
            || root.digest(scope.keyspace()).ok().as_deref()
                != Some(approved_external_root_blake3)
        {
            return Err(S3V2TrustAnchorError::InvalidExternalPin);
        }
        if !valid_etag(etag) || !verified.attests_exact_root(scope, root, etag) {
            return Err(S3V2TrustAnchorError::UnverifiedRead);
        }
        let root_json = root.encode(scope.keyspace())
            .map_err(|_| S3V2TrustAnchorError::InvalidExternalPin)?;
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(old) = Self::load_from(&tx, scope)? {
            if old.root == *root && old.root_etag == etag
                && old.previous_root.is_none()
            {
                tx.commit()?;
                return Ok(S3V2TrustAnchorOutcome::Unchanged);
            }
            return Err(S3V2TrustAnchorError::RootRewritten);
        }
        let digest = receipt_digest(scope, &root_json, etag, None);
        tx.execute(
            "INSERT INTO s3_v2_trusted_roots (
                profile_id,vault_id,scope_fingerprint,head_key,head_json,
                head_etag,previous_head_json,receipt_digest,record_schema
             ) VALUES (?1,?2,?3,?4,?5,?6,NULL,?7,1)",
            params![
                scope.profile_id(),
                scope.keyspace().vault_id().to_string(),
                scope.binding_fingerprint(), scope.keyspace().current_head_key(),
                root_json, etag, digest,
            ],
        )?;
        tx.commit()?;
        Ok(S3V2TrustAnchorOutcome::Inserted)
    }

    /// Advance by exactly ONE fully-verified direct V2 successor. A missing,
    /// skipped, old, rewritten or forked generation is Hold. Private SQLite
    /// receipt mutation does NOT change any user document baseline.
    pub fn record_verified_successor(
        &mut self,
        scope: &S3RemoteAnchorScope,
        root: &S3PackedHeadV2,
        etag: &str,
        verified: &S3PinnedV2ReadPreview,
    ) -> Result<S3V2TrustAnchorOutcome, S3V2TrustAnchorError> {
        if !valid_etag(etag) || !verified.attests_exact_root(scope, root, etag) {
            return Err(S3V2TrustAnchorError::UnverifiedRead);
        }
        let ns = scope.keyspace();
        let root_json = root.encode(ns)
            .map_err(|_| S3V2TrustAnchorError::RollbackOrFork)?;
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old = Self::load_from(&tx, scope)?
            .ok_or(S3V2TrustAnchorError::MissingTrustedAnchor)?;
        if old.root == *root {
            if old.root_etag != etag {
                return Err(S3V2TrustAnchorError::RootRewritten);
            }
            tx.commit()?;
            return Ok(S3V2TrustAnchorOutcome::Unchanged);
        }
        // A Vault with an enrolled owner/publisher policy must NEVER be
        // advanced via the legacy "pinned bytes only" API: signed publisher
        // successor authorization has not been implemented yet. Without this
        // gate an untrusted S3 server could offer a complete but UNSIGNED
        // direct successor and trick an old caller into advancing the
        // locally trusted root. Existing un-enrolled research roots retain
        // their original standalone pinned-read behavior.
        #[cfg(feature = "signed-vault-atomic-join-lab")]
        {
            let policy_present: i64 = tx.query_row(
                "SELECT CASE WHEN policy_json IS NULL THEN 0 ELSE 1 END
                   FROM s3_v2_trusted_roots WHERE profile_id=?1",
                params![scope.profile_id()],
                |row| row.get(0),
            )?;
            if policy_present != 0 {
                return Err(S3V2TrustAnchorError::UnverifiedRead);
            }
        }
        if root.generation() <= old.root.generation() {
            return Err(S3V2TrustAnchorError::RollbackOrFork);
        }
        if root.generation() != old.root.generation() + 1
            || !root.is_direct_successor_of(ns, &old.root)
        {
            return Err(S3V2TrustAnchorError::IncompleteAncestry);
        }
        let prev_bytes = old.root.encode(ns)
            .map_err(|_| S3V2TrustAnchorError::CorruptedAnchor)?;
        let old_bytes = prev_bytes.clone();
        let previous_before = old.previous_root.as_ref()
            .map(|old| old.encode(ns)).transpose()
            .map_err(|_| S3V2TrustAnchorError::CorruptedAnchor)?;
        let old_receipt = receipt_digest(
            scope, &old_bytes, &old.root_etag, previous_before.as_deref(),
        );
        let digest = receipt_digest(scope, &root_json, etag, Some(&prev_bytes));
        let changed = tx.execute(
            "UPDATE s3_v2_trusted_roots SET
                head_json=?2,head_etag=?3,previous_head_json=?4,
                receipt_digest=?5
             WHERE profile_id=?1 AND receipt_digest=?6
               AND scope_fingerprint=?7",
            params![
                scope.profile_id(), root_json, etag, prev_bytes,
                digest, old_receipt, scope.binding_fingerprint(),
            ],
        )?;
        if changed != 1 {
            return Err(S3V2TrustAnchorError::RollbackOrFork);
        }
        tx.commit()?;
        Ok(S3V2TrustAnchorOutcome::Advanced)
    }

    /// Reopen a persisted pin and fully re-verify the exact same current
    /// remote V2 root/ETag. Refuses absent, changed or foreign roots; this
    /// does NOT automatically persist a new generation or stage files.
    pub async fn inspect_saved_root(
        &self,
        client: &AwsS3ProbeClient,
        scope: &S3RemoteAnchorScope,
    ) -> Result<S3PinnedV2ReadPreview, S3V2TrustAnchorError> {
        let old = self.load(scope)?
            .ok_or(S3V2TrustAnchorError::MissingTrustedAnchor)?;
        client.inspect_explicitly_pinned_v2_read_only(
            scope, old.root(), old.etag(), old.previous_root(),
        ).await.map_err(S3V2TrustAnchorError::Reader)
    }
}

// The policy checkpoint shares this existing guarded SQLite connection and
// V2 root row. No second policy journal, separate path, or S3 writer.
#[cfg(feature = "signed-vault-policy-receipt-lab")]
#[path = "s3_v2_trust_anchor_policy_receipt_lab.rs"]
mod policy_receipt_lab;

#[cfg(feature = "signed-vault-policy-receipt-lab")]
pub use policy_receipt_lab::{
    PolicyReceiptOutcome, V2PolicyReceiptError, V2TrustedPublisherPolicy,
};
#[cfg(feature = "signed-vault-atomic-join-lab")]
pub use policy_receipt_lab::{AtomicJoinError, AtomicJoinOutcome};

#[cfg(test)]
#[path = "s3_v2_trust_anchor_tests.rs"]
mod tests;
