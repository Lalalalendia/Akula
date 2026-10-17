//! SYNC-DELTA-01i: profile-bound, crash-durable, read-only metadata *hints*.
//!
//! Cold-start reuse is advisory only. The saved revision JSON was fully
//! verified when this cache was created. Reopening the private SQLite cache
//! recomputes each immutable JSON BLAKE3 and the complete revision DAG, but
//! DOES NOT re-prove present-day S3 blob availability or writer authority.
//! Never use this cache to authorize File Gate writes, delete/GC, publish,
//! advance a per-document baseline, or claim that sync has completed.
//!
//! SQLite and an unkeyed BLAKE3 checksum do not defend against a malicious
//! local database replacement. The caller must select a protected app-data
//! path outside the user's Markdown vault. All persisted data is metadata:
//! revision paths and content hashes can still be sensitive.

use std::path::Path;

use blake3::Hasher;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::{
    AwsS3ProbeClient, S3SelectiveReadError, S3SelectiveRevisionPreview,
    S3VerifiedRevisionMetadataStore, MAX_S3_REVISION_METADATA_CACHE_BYTES,
};
use crate::sync::{
    S3ManifestDeltaLimits, S3ManifestV1, S3RemoteAnchorError,
    S3RemoteAnchorJournal, S3RemoteAnchorScope, S3RevisionRefV1,
    MAX_REVISIONS,
};

const SCHEMA_LABEL: &[u8] = b"akula-read-only-s3-metadata-hints-v1";
const MAX_REFERENCE_JSON: usize = 512;
const MAX_REVISION_JSON: usize = 64 * 1024;

#[derive(Debug)]
pub enum S3VerifiedHintsError {
    Sqlite(rusqlite::Error),
    Anchor(S3RemoteAnchorError),
    NoVerifiedAnchor,
    WrongConnectedProfile,
    StaleRemoteAnchor,
    InvalidVerifiedInput,
    CorruptPersistedMetadata,
    BudgetExceeded,
    Selective(S3SelectiveReadError),
}

impl From<rusqlite::Error> for S3VerifiedHintsError {
    fn from(err: rusqlite::Error) -> Self { Self::Sqlite(err) }
}
impl From<S3RemoteAnchorError> for S3VerifiedHintsError {
    fn from(err: S3RemoteAnchorError) -> Self { Self::Anchor(err) }
}
impl From<S3SelectiveReadError> for S3VerifiedHintsError {
    fn from(err: S3SelectiveReadError) -> Self { Self::Selective(err) }
}

fn include(hasher: &mut Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn initial_digest(
    scope: &S3RemoteAnchorScope,
    snapshot: &str,
    count: usize,
    bytes: usize,
) -> Hasher {
    let mut hasher = Hasher::new();
    for field in [
        SCHEMA_LABEL,
        scope.binding_fingerprint().as_bytes(),
        snapshot.as_bytes(),
        &(count as u64).to_le_bytes(),
        &(bytes as u64).to_le_bytes(),
    ] {
        include(&mut hasher, field);
    }
    hasher
}

fn row_digest(
    hasher: &mut Hasher,
    revision_id: &str,
    reference_bytes: &[u8],
    json: &[u8],
) {
    include(hasher, revision_id.as_bytes());
    include(hasher, reference_bytes);
    include(hasher, json);
}

fn anchor_identity(
    scope: &S3RemoteAnchorScope,
    anchor_journal: &S3RemoteAnchorJournal,
) -> Result<(String, usize), S3VerifiedHintsError> {
    let anchor = anchor_journal.load(scope)?
        .ok_or(S3VerifiedHintsError::NoVerifiedAnchor)?;
    let manifest = S3ManifestV1::decode(
        scope.keyspace(),
        anchor.pointer().manifest_id(),
        anchor.manifest_bytes(),
    ).map_err(|_| S3VerifiedHintsError::CorruptPersistedMetadata)?;
    let id = format!(
        "{}:{}",
        anchor.pointer().manifest_id(),
        anchor.pointer().manifest_blake3(),
    );
    Ok((id, manifest.total_revisions as usize))
}

/// Dedicated SQLite file in private app-data. Never shares a table with the
/// native document operation journal, and cannot write to S3 or the user vault.
pub struct S3VerifiedHintsJournal {
    db: Connection,
}

impl S3VerifiedHintsJournal {
    pub fn open(path: &Path) -> Result<Self, S3VerifiedHintsError> {
        let db = Connection::open(path)?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;
             CREATE TABLE IF NOT EXISTS s3_metadata_hint_header_v1 (
                 profile_id TEXT NOT NULL,
                 vault_id TEXT NOT NULL,
                 scope_fingerprint TEXT NOT NULL,
                 snapshot_id TEXT NOT NULL,
                 revision_count INTEGER NOT NULL,
                 json_bytes INTEGER NOT NULL,
                 receipt_digest TEXT NOT NULL,
                 PRIMARY KEY(profile_id, vault_id)
             );
             CREATE TABLE IF NOT EXISTS s3_metadata_hint_entry_v1 (
                 profile_id TEXT NOT NULL,
                 vault_id TEXT NOT NULL,
                 revision_id TEXT NOT NULL,
                 reference_json BLOB NOT NULL,
                 revision_json BLOB NOT NULL,
                 PRIMARY KEY(profile_id, vault_id, revision_id),
                 FOREIGN KEY(profile_id, vault_id)
                    REFERENCES s3_metadata_hint_header_v1(profile_id, vault_id)
                    ON DELETE CASCADE
             );",
        )?;
        Ok(Self { db })
    }

    /// Persist ONLY an already completely verified in-memory S3 revision
    /// metadata snapshot against the exact SQLite remote observation anchor.
    /// No new network verification, File Gate operation or baseline commit.
    ///
    /// Crash safety: writer is serialized by BEGIN IMMEDIATE; header and all
    /// entries replace the prior version within one FULL-synchronous commit.
    /// A crash can leave the old version but never half of the new one.
    pub fn persist_after_full_pin(
        &mut self,
        scope: &S3RemoteAnchorScope,
        anchor_journal: &S3RemoteAnchorJournal,
        verified: &S3VerifiedRevisionMetadataStore,
    ) -> Result<(), S3VerifiedHintsError> {
        let (snapshot, expected_count) = anchor_identity(scope, anchor_journal)?;
        if !verified.is_verified_for(scope.keyspace().vault_id(), &snapshot)
            || verified.len() != expected_count
        {
            return Err(S3VerifiedHintsError::InvalidVerifiedInput);
        }
        if verified.len() > MAX_REVISIONS
            || verified.total_bytes() > MAX_S3_REVISION_METADATA_CACHE_BYTES
        {
            return Err(S3VerifiedHintsError::BudgetExceeded);
        }
        let profile_id = scope.profile_id();
        let vault = scope.keyspace().vault_id().to_string();
        let fingerprint = scope.binding_fingerprint();

        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = tx.query_row(
            "SELECT scope_fingerprint FROM s3_metadata_hint_header_v1
             WHERE profile_id=?1 AND vault_id=?2",
            params![profile_id, vault],
            |row| row.get::<_, String>(0),
        ).optional()?;
        if existing.as_deref().is_some_and(|old| old != fingerprint) {
            return Err(S3VerifiedHintsError::WrongConnectedProfile);
        }

        tx.execute(
            "DELETE FROM s3_metadata_hint_header_v1
             WHERE profile_id=?1 AND vault_id=?2",
            params![profile_id, vault],
        )?;
        tx.execute(
            "INSERT INTO s3_metadata_hint_header_v1
             (profile_id, vault_id, scope_fingerprint, snapshot_id,
              revision_count, json_bytes, receipt_digest)
             VALUES (?1,?2,?3,?4,?5,?6,'')",
            params![
                profile_id, vault, fingerprint, snapshot,
                verified.len() as i64, verified.total_bytes() as i64,
            ],
        )?;
        let mut digest = initial_digest(
            scope, &snapshot, verified.len(), verified.total_bytes(),
        );
        {
            let mut insert = tx.prepare(
                "INSERT INTO s3_metadata_hint_entry_v1
                 (profile_id, vault_id, revision_id, reference_json, revision_json)
                 VALUES (?1,?2,?3,?4,?5)"
            )?;
            for (reference, json) in verified.verified_entries() {
                let encoded_ref = serde_json::to_vec(reference)
                    .map_err(|_| S3VerifiedHintsError::InvalidVerifiedInput)?;
                if encoded_ref.len() > MAX_REFERENCE_JSON
                    || json.len() > MAX_REVISION_JSON
                {
                    return Err(S3VerifiedHintsError::BudgetExceeded);
                }
                let id = reference.revision_id.to_string();
                row_digest(&mut digest, &id, &encoded_ref, json);
                insert.execute(params![
                    profile_id, vault, id, encoded_ref, json,
                ])?;
            }
        }
        tx.execute(
            "UPDATE s3_metadata_hint_header_v1 SET receipt_digest=?3
             WHERE profile_id=?1 AND vault_id=?2",
            params![profile_id, vault, digest.finalize().to_hex().to_string()],
        )?;
        tx.commit()?;

        // A separate process might advance the remote anchor in its own
        // SQLite DB. If so, this old cache remains inert until explicitly
        // re-pinned and persisted for that newer anchor.
        let (after, _) = anchor_identity(scope, anchor_journal)?;
        if after != snapshot {
            return Err(S3VerifiedHintsError::StaleRemoteAnchor);
        }
        Ok(())
    }

    /// Internal only: returning a read-only RemoteStore-like snapshot as a
    /// public cache API could let a future caller confuse old hints with a
    /// newly pinned, currently available remote. Expose only advisory preview
    /// through AwsS3ProbeClient::inspect_selective_from_persisted_hints.
    fn load_hints(
        &self,
        scope: &S3RemoteAnchorScope,
        anchor_journal: &S3RemoteAnchorJournal,
        max_metadata_bytes: usize,
    ) -> Result<Option<S3VerifiedRevisionMetadataStore>, S3VerifiedHintsError> {
        if max_metadata_bytes == 0
            || max_metadata_bytes > MAX_S3_REVISION_METADATA_CACHE_BYTES
        {
            return Err(S3VerifiedHintsError::BudgetExceeded);
        }
        let (anchor_snapshot, anchor_count) = anchor_identity(scope, anchor_journal)?;
        let profile_id = scope.profile_id();
        let vault = scope.keyspace().vault_id().to_string();
        let header = self.db.query_row(
            "SELECT scope_fingerprint,snapshot_id,revision_count,
                    json_bytes,receipt_digest
             FROM s3_metadata_hint_header_v1
             WHERE profile_id=?1 AND vault_id=?2",
            params![profile_id, vault],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            )),
        ).optional()?;
        let Some((fingerprint,saved_snapshot,count,bytes,receipt)) = header else {
            return Ok(None);
        };
        if fingerprint != scope.binding_fingerprint() {
            return Err(S3VerifiedHintsError::WrongConnectedProfile);
        }
        if saved_snapshot != anchor_snapshot {
            return Err(S3VerifiedHintsError::StaleRemoteAnchor);
        }
        if count < 0 || count as usize > MAX_REVISIONS
            || count as usize != anchor_count
            || bytes < 0 || bytes as usize > max_metadata_bytes
            || receipt.len() != 64
        {
            return Err(S3VerifiedHintsError::CorruptPersistedMetadata);
        }
        let (row_count,total_bytes,largest_json,largest_reference): (i64,i64,i64,i64) =
            self.db.query_row(
                "SELECT COUNT(*),COALESCE(SUM(length(revision_json)),0),
                        COALESCE(MAX(length(revision_json)),0),
                        COALESCE(MAX(length(reference_json)),0)
                 FROM s3_metadata_hint_entry_v1
                 WHERE profile_id=?1 AND vault_id=?2",
                params![profile_id,vault],
                |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
            )?;
        // SQL length checks run BEFORE allocating any per-row BLOB. A corrupt
        // database cannot force us to read an unbounded revision payload.
        if row_count != count || total_bytes != bytes
            || largest_json > MAX_REVISION_JSON as i64
            || largest_reference > MAX_REFERENCE_JSON as i64
        {
            return Err(S3VerifiedHintsError::CorruptPersistedMetadata);
        }
        let mut rebuilt = S3VerifiedRevisionMetadataStore::new(
            scope.keyspace().vault_id(), saved_snapshot.clone(), max_metadata_bytes,
        ).map_err(|_| S3VerifiedHintsError::CorruptPersistedMetadata)?;
        let mut hasher = initial_digest(
            scope, &saved_snapshot, count as usize, bytes as usize,
        );
        let mut statement = self.db.prepare(
            "SELECT revision_id,reference_json,revision_json
             FROM s3_metadata_hint_entry_v1
             WHERE profile_id=?1 AND vault_id=?2
             ORDER BY revision_id"
        )?;
        let mut rows = statement.query(params![profile_id,vault])?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let reference_bytes: Vec<u8> = row.get(1)?;
            let json: Vec<u8> = row.get(2)?;
            let reference: S3RevisionRefV1 =
                serde_json::from_slice(&reference_bytes)
                    .map_err(|_| S3VerifiedHintsError::CorruptPersistedMetadata)?;
            if id != reference.revision_id.to_string()
                || serde_json::to_vec(&reference).ok().as_deref()
                    != Some(reference_bytes.as_slice())
            {
                return Err(S3VerifiedHintsError::CorruptPersistedMetadata);
            }
            row_digest(&mut hasher, &id, &reference_bytes, &json);
            rebuilt.accept(reference, json)
                .map_err(|_| S3VerifiedHintsError::CorruptPersistedMetadata)?;
        }
        drop(rows);
        drop(statement);
        if hasher.finalize().to_hex().as_str() != receipt
            || rebuilt.len() != count as usize
            || rebuilt.total_bytes() != bytes as usize
        {
            return Err(S3VerifiedHintsError::CorruptPersistedMetadata);
        }
        rebuilt.validate_complete_dag()
            .map_err(|_| S3VerifiedHintsError::CorruptPersistedMetadata)?;
        Ok(Some(rebuilt))
    }
}

impl AwsS3ProbeClient {
    /// Cold-start advisory read. A caller cannot obtain a writable provider
    /// or full-pin authority from the persisted SQLite hint cache.
    ///
    /// Even after a structurally valid BLAKE3/DAG reload, old blobs may no
    /// longer exist on S3. This preview MUST NOT authorize native file writes.
    pub async fn inspect_selective_from_persisted_hints(
        &self,
        scope: &S3RemoteAnchorScope,
        remote_anchor: &S3RemoteAnchorJournal,
        hints: &S3VerifiedHintsJournal,
        limits: S3ManifestDeltaLimits,
    ) -> Result<S3SelectiveRevisionPreview, S3VerifiedHintsError> {
        if !scope.matches_connection(self.bound_profile(), self.bound_keyspace()) {
            return Err(S3VerifiedHintsError::WrongConnectedProfile);
        }
        let restored = hints.load_hints(
            scope, remote_anchor, MAX_S3_REVISION_METADATA_CACHE_BYTES,
        )?.ok_or(S3VerifiedHintsError::InvalidVerifiedInput)?;
        self.inspect_selective_s3_revision_changes(
            scope, remote_anchor, &restored,
            MAX_S3_REVISION_METADATA_CACHE_BYTES, limits,
        ).await.map_err(S3VerifiedHintsError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{
        OperationId, S3ConnectionProfile, S3HeadPointerV1,
        S3Keyspace, S3ManifestV1, S3PinnedRevisionSnapshot,
        S3VerifiedHead, VaultId,
    };
    use tempfile::tempdir;

    struct Fixture {
        _directory: tempfile::TempDir,
        pub scope: S3RemoteAnchorScope,
        pub anchor: S3RemoteAnchorJournal,
        pub hints: S3VerifiedHintsJournal,
        pub verified: S3VerifiedRevisionMetadataStore,
    }

    fn test_profile() -> S3ConnectionProfile {
        S3ConnectionProfile {
            bucket: "private-ci-cache".into(),
            region: "us-east-1".into(),
            endpoint: Some("http://127.0.0.1:9000".into()),
            force_path_style: true,
        }
    }

    fn genesis() -> Fixture {
        let dir = tempdir().unwrap();
        let keyspace = S3Keyspace::new("akula/hints", VaultId::new()).unwrap();
        let scope = S3RemoteAnchorScope::new(
            "desktop", "independently-verified-deployment",
            &test_profile(), &keyspace,
        ).unwrap();
        let mut anchor = S3RemoteAnchorJournal::open(
            &dir.path().join("remote.sqlite"),
        ).unwrap();
        let manifest = S3ManifestV1::new(
            &keyspace, OperationId::new(), 1, None, vec![],
        ).unwrap();
        let bytes = manifest.encode(&keyspace).unwrap();
        let digest = blake3::hash(&bytes).to_hex().to_string();
        let pointer = S3HeadPointerV1::new(
            &keyspace, 1, manifest.manifest_id, &digest, None,
        ).unwrap();
        let id = format!("{}:{digest}", manifest.manifest_id);
        let pinned = S3PinnedRevisionSnapshot::for_remote_anchor_test(id.clone(), 0);
        anchor.record_verified_remote(
            &scope,
            &S3VerifiedHead {
                pointer, etag:"\"ci-etag1\"".into(), manifest,
            },
            &pinned, &[bytes],
        ).unwrap();
        let mut verified = S3VerifiedRevisionMetadataStore::new(
            keyspace.vault_id(), id, 1024*1024,
        ).unwrap();
        verified.validate_complete_dag().unwrap();
        let hints = S3VerifiedHintsJournal::open(
            &dir.path().join("hints.sqlite"),
        ).unwrap();
        Fixture {_directory:dir,scope,anchor,hints,verified}
    }

    #[test]
    fn genesis_cache_recovers_idempotently_and_is_read_only() {
        let mut f = genesis();
        f.hints.persist_after_full_pin(
            &f.scope, &f.anchor, &f.verified,
        ).unwrap();
        let recovered = f.hints.load_hints(
            &f.scope, &f.anchor, 1024*1024,
        ).unwrap().unwrap();
        assert_eq!(recovered.len(),0);
        assert_eq!(recovered.snapshot_id(),f.verified.snapshot_id());
        f.hints.persist_after_full_pin(
            &f.scope, &f.anchor, &f.verified,
        ).unwrap();
        assert!(f.hints.load_hints(&f.scope,&f.anchor,1024*1024).unwrap().is_some());
    }

    #[test]
    fn wrong_bucket_or_account_binding_rejects_record() {
        let mut f=genesis();
        f.hints.persist_after_full_pin(&f.scope,&f.anchor,&f.verified).unwrap();
        let ns=f.scope.keyspace();
        let mut different=test_profile();
        different.bucket="other-bucket".into();
        for scope in [
            S3RemoteAnchorScope::new(
                "desktop","independently-verified-deployment",&different,ns,
            ).unwrap(),
            S3RemoteAnchorScope::new(
                "desktop","different-account",&test_profile(),ns,
            ).unwrap(),
        ] {
            assert!(matches!(
                f.hints.load_hints(&scope,&f.anchor,1024*1024),
                Err(S3VerifiedHintsError::Anchor(S3RemoteAnchorError::ScopeChanged))
            ));
        }
    }

    #[test]
    fn corrupted_receipt_and_oversized_row_are_never_reused() {
        let mut f=genesis();
        f.hints.persist_after_full_pin(&f.scope,&f.anchor,&f.verified).unwrap();
        f.hints.db.execute(
            "UPDATE s3_metadata_hint_header_v1 SET receipt_digest='invalid'",
            [],
        ).unwrap();
        assert!(matches!(
            f.hints.load_hints(&f.scope,&f.anchor,1024*1024),
            Err(S3VerifiedHintsError::CorruptPersistedMetadata),
        ));
        f.hints.persist_after_full_pin(&f.scope,&f.anchor,&f.verified).unwrap();
        f.hints.db.execute(
            "INSERT INTO s3_metadata_hint_entry_v1
             (profile_id,vault_id,revision_id,reference_json,revision_json)
             VALUES (?1,?2,'fake-id',?3,?4)",
            params![
                f.scope.profile_id(),f.scope.keyspace().vault_id().to_string(),
                vec![b'x'; MAX_REFERENCE_JSON + 1],b"{}".to_vec(),
            ],
        ).unwrap();
        assert!(matches!(
            f.hints.load_hints(&f.scope,&f.anchor,1024*1024),
            Err(S3VerifiedHintsError::CorruptPersistedMetadata),
        ));
    }

    #[test]
    fn stale_remote_root_blocks_cache_after_restart() {
        let mut f=genesis();
        f.hints.persist_after_full_pin(&f.scope,&f.anchor,&f.verified).unwrap();
        let keyspace=f.scope.keyspace();
        let old=f.anchor.load(&f.scope).unwrap().unwrap();
        let manifest=S3ManifestV1::new(
            keyspace,OperationId::new(),2,
            Some((old.pointer().manifest_id(),
                old.pointer().manifest_blake3().into())),vec![],
        ).unwrap();
        let bytes=manifest.encode(keyspace).unwrap();
        let hash=blake3::hash(&bytes).to_hex().to_string();
        let pointer=S3HeadPointerV1::new(
            keyspace,2,manifest.manifest_id,&hash,
            Some(old.pointer().manifest_id()),
        ).unwrap();
        let pinned=S3PinnedRevisionSnapshot::for_remote_anchor_test(
            format!("{}:{hash}",manifest.manifest_id),0,
        );
        f.anchor.record_verified_remote(
            &f.scope,&S3VerifiedHead {
                pointer,manifest,etag:"\"ci-etag2\"".into(),
            },&pinned,&[bytes],
        ).unwrap();
        assert!(matches!(
            f.hints.load_hints(&f.scope,&f.anchor,1024*1024),
            Err(S3VerifiedHintsError::StaleRemoteAnchor),
        ));
    }

    #[test]
    fn missing_verified_anchor_and_strict_budget_fail_closed() {
        let f=genesis();
        assert!(f.hints.load_hints(&f.scope,&f.anchor,1024*1024).unwrap().is_none());
        assert!(matches!(
            f.hints.load_hints(&f.scope,&f.anchor,0),
            Err(S3VerifiedHintsError::BudgetExceeded),
        ));
    }
}
