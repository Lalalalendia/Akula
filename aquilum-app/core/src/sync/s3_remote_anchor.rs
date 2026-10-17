//! SYNC-DELTA-01e: durable, profile-bound REMOTE observation anchors.
//!
//! These anchors support read-only S3 delta discovery. They are deliberately
//! NOT document ConfirmedBaseline records and never authorize File Gate writes,
//! tombstone application, remote deletion, CAS publication or journal commits.
//!
//! Every insertion/advance requires a caller-supplied fully pinned read-only
//! revision snapshot and an exact, verified canonical manifest ancestry.
//! The pinned snapshot must come from the existing S3 pinning pipeline; this
//! module does not independently fetch blobs or authenticate the remote writer.
//!
//! Local SQLite and BLAKE3 are crash/integrity tools, NOT tamper-proof storage:
//! a malicious actor able to replace the database or the remote head can
//! defeat anti-rollback until a separately trusted monotonic anchor is added.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::{
    S3ConnectionProfile, S3ContractError, S3HeadPointerV1, S3Keyspace,
    S3ManifestDeltaBase, S3ManifestDeltaError, S3ManifestDeltaLimits,
    S3ManifestDeltaOutcome, S3ManifestError, S3ManifestV1,
    S3PinnedRevisionSnapshot, S3TransportError, S3VerifiedHead, VaultId,
    inspect_s3_manifest_delta,
};

const MAX_SCOPE_LABEL: usize = 128;
const MAX_ANCHOR_ETAG: usize = 256;
const DB_SCHEMA: &str = "s3-remote-anchor-v1";

#[derive(Debug)]
pub enum S3RemoteAnchorError {
    Sqlite(rusqlite::Error),
    Transport(S3TransportError),
    Contract(S3ContractError),
    Manifest(S3ManifestError),
    Delta(S3ManifestDeltaError),
    InvalidScope,
    ScopeChanged,
    InvalidEvidence,
    CorruptAnchor,
    VersionRewritten,
    RollbackOrFork,
    IncompleteAncestry,
    VerifiedSnapshotRequired,
}

impl From<rusqlite::Error> for S3RemoteAnchorError {
    fn from(value: rusqlite::Error) -> Self { Self::Sqlite(value) }
}
impl From<S3TransportError> for S3RemoteAnchorError {
    fn from(value: S3TransportError) -> Self { Self::Transport(value) }
}
impl From<S3ContractError> for S3RemoteAnchorError {
    fn from(value: S3ContractError) -> Self { Self::Contract(value) }
}
impl From<S3ManifestError> for S3RemoteAnchorError {
    fn from(value: S3ManifestError) -> Self { Self::Manifest(value) }
}
impl From<S3ManifestDeltaError> for S3RemoteAnchorError {
    fn from(value: S3ManifestDeltaError) -> Self { Self::Delta(value) }
}

fn valid_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SCOPE_LABEL
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'-' | b'_' | b'.' | b':' )
        })
}

fn valid_etag(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ANCHOR_ETAG
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

/// Bound to a trusted non-secret stable account identity as well as the
/// bucket/region/endpoint/path-style and vault keyspace. The account binding
/// MUST be verified by the caller/credential provider: an arbitrary supplied
/// string alone does not prove AWS principal, ownership or writer authority.
///
/// A stable app profile ID reused for another bucket/endpoint/account fails
/// closed rather than silently treating a different remote as the old vault.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3RemoteAnchorScope {
    profile_id: String,
    vault_id: VaultId,
    fingerprint: String,
    keyspace: S3Keyspace,
}

impl S3RemoteAnchorScope {
    pub fn new(
        profile_id: &str,
        verified_account_binding: &str,
        profile: &S3ConnectionProfile,
        keyspace: &S3Keyspace,
    ) -> Result<Self, S3RemoteAnchorError> {
        if !valid_label(profile_id) || !valid_label(verified_account_binding) {
            return Err(S3RemoteAnchorError::InvalidScope);
        }
        profile.validate()?;
        let bytes = serde_json::to_vec(&(
            DB_SCHEMA,
            profile_id,
            verified_account_binding,
            &profile.bucket,
            &profile.region,
            &profile.endpoint,
            profile.force_path_style,
            keyspace.current_head_key(),
        ))
        .map_err(|_| S3RemoteAnchorError::InvalidScope)?;
        Ok(Self {
            profile_id: profile_id.into(),
            vault_id: keyspace.vault_id(),
            fingerprint: blake3::hash(&bytes).to_hex().to_string(),
            keyspace: keyspace.clone(),
        })
    }

    pub fn keyspace(&self) -> &S3Keyspace { &self.keyspace }
    pub fn profile_id(&self) -> &str { &self.profile_id }
}

/// An internally checked, durable, immutable remote head and manifest pair.
/// It is NOT an authenticated writer identity or a local document baseline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3RemoteAnchor {
    pointer: S3HeadPointerV1,
    etag: String,
    manifest_bytes: Vec<u8>,
}

impl S3RemoteAnchor {
    pub fn pointer(&self) -> &S3HeadPointerV1 { &self.pointer }
    pub fn etag(&self) -> &str { &self.etag }
    pub fn manifest_bytes(&self) -> &[u8] { &self.manifest_bytes }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum S3RemoteAnchorOutcome {
    Inserted,
    Advanced,
    Unchanged,
}

/// Separate private-app-data SQLite database. This never touches the user's
/// Markdown vault or the existing per-document sync journal. BEGIN IMMEDIATE
/// serializes concurrent checkpoint writers and rejects generation rollback.
pub struct S3RemoteAnchorJournal {
    db: Connection,
}

impl S3RemoteAnchorJournal {
    /// The caller MUST select a protected, private app-data path.
    pub fn open(path: &Path) -> Result<Self, S3RemoteAnchorError> {
        let db = Connection::open(path)?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;
             CREATE TABLE IF NOT EXISTS s3_remote_anchor_v1 (
                 profile_id TEXT NOT NULL,
                 vault_id TEXT NOT NULL,
                 scope_fingerprint TEXT NOT NULL,
                 head_json BLOB NOT NULL,
                 head_etag TEXT NOT NULL,
                 manifest_bytes BLOB NOT NULL,
                 receipt_digest TEXT NOT NULL,
                 PRIMARY KEY(profile_id, vault_id)
             );",
        )?;
        Ok(Self { db })
    }

    pub fn load(
        &self,
        scope: &S3RemoteAnchorScope,
    ) -> Result<Option<S3RemoteAnchor>, S3RemoteAnchorError> {
        Self::load_from(&self.db, scope)
    }

    fn load_from(
        db: &Connection,
        scope: &S3RemoteAnchorScope,
    ) -> Result<Option<S3RemoteAnchor>, S3RemoteAnchorError> {
        let row = db.query_row(
            "SELECT scope_fingerprint,head_json,head_etag,manifest_bytes,receipt_digest
             FROM s3_remote_anchor_v1 WHERE profile_id=?1 AND vault_id=?2",
            params![scope.profile_id, scope.vault_id.to_string()],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, String>(4)?,
            )),
        ).optional()?;
        let Some((fingerprint, head_json, etag, manifest_bytes, receipt)) = row else {
            return Ok(None);
        };
        if fingerprint != scope.fingerprint {
            return Err(S3RemoteAnchorError::ScopeChanged);
        }
        if !valid_etag(&etag)
            || receipt != record_digest(&fingerprint, &head_json, &etag, &manifest_bytes)
        {
            return Err(S3RemoteAnchorError::CorruptAnchor);
        }
        let pointer = S3HeadPointerV1::decode(&head_json, &scope.keyspace)
            .map_err(|_| S3RemoteAnchorError::CorruptAnchor)?;
        let manifest = S3ManifestV1::decode(
            &scope.keyspace,
            pointer.manifest_id(),
            &manifest_bytes,
        ).map_err(|_| S3RemoteAnchorError::CorruptAnchor)?;
        if pointer.encode(&scope.keyspace).ok().as_deref() != Some(head_json.as_slice())
            || manifest.encode(&scope.keyspace).ok().as_deref()
                != Some(manifest_bytes.as_slice())
            || blake3::hash(&manifest_bytes).to_hex().as_str()
                != pointer.manifest_blake3()
            || manifest.generation != pointer.generation()
            || manifest.previous_manifest_id != pointer.previous_manifest_id()
        {
            return Err(S3RemoteAnchorError::CorruptAnchor);
        }
        Ok(Some(S3RemoteAnchor {
            pointer,
            etag,
            manifest_bytes,
        }))
    }

    /// Persist a new *remote observation anchor*, never a document baseline.
    ///
    /// The upstream S3PinnedRevisionSnapshot must originate from the existing
    /// fully verified, bounded pin_revision_snapshot pipeline (immutable
    /// shards, entire DAG, referenced blobs and ancestor retention checks).
    /// A source/head race must have been independently ruled out by the caller.
    ///
    /// For initial adoption provide a complete raw manifest chain to genesis;
    /// for an update provide exact raw immutable bytes newest-first up to the
    /// stored anchor. The pure inspector proves no ancestor was skipped.
    ///
    /// Missing/expired ancestry returns Hold: it does not overwrite the old
    /// trusted checkpoint or synthesize an empty/deleted remote.
    pub fn record_verified_remote(
        &mut self,
        scope: &S3RemoteAnchorScope,
        verified: &S3VerifiedHead,
        pinned: &S3PinnedRevisionSnapshot,
        newest_first: &[Vec<u8>],
    ) -> Result<S3RemoteAnchorOutcome, S3RemoteAnchorError> {
        if !valid_etag(&verified.etag) {
            return Err(S3RemoteAnchorError::InvalidEvidence);
        }

        let head_json = verified.pointer.encode(&scope.keyspace)?;
        let manifest_bytes = verified.manifest.encode(&scope.keyspace)?;
        let expected_snapshot_id = format!(
            "{}:{}",
            verified.pointer.manifest_id(),
            verified.pointer.manifest_blake3(),
        );
        if verified.manifest.manifest_id != verified.pointer.manifest_id()
            || verified.manifest.generation != verified.pointer.generation()
            || verified.manifest.previous_manifest_id
                != verified.pointer.previous_manifest_id()
            || blake3::hash(&manifest_bytes).to_hex().as_str()
                != verified.pointer.manifest_blake3()
            || pinned.snapshot_id != expected_snapshot_id
            || pinned.len() != verified.manifest.total_revisions as usize
        {
            return Err(S3RemoteAnchorError::InvalidEvidence);
        }

        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous = Self::load_from(&tx, scope)?;
        if let Some(prior) = &previous {
            if verified.pointer == prior.pointer {
                if verified.etag != prior.etag || manifest_bytes != prior.manifest_bytes
                    || !newest_first.is_empty()
                {
                    return Err(S3RemoteAnchorError::VersionRewritten);
                }
                tx.commit()?;
                return Ok(S3RemoteAnchorOutcome::Unchanged);
            }
            if verified.pointer.generation() <= prior.pointer.generation() {
                return Err(S3RemoteAnchorError::RollbackOrFork);
            }
        }

        if newest_first.first().map(Vec::as_slice) != Some(manifest_bytes.as_slice()) {
            return Err(S3RemoteAnchorError::IncompleteAncestry);
        }
        let prior_pair = previous.as_ref().map(|prior| {
            (&prior.pointer, prior.manifest_bytes.as_slice())
        });
        let result = inspect_s3_manifest_delta(
            &scope.keyspace,
            &verified.pointer,
            prior_pair,
            newest_first,
            S3ManifestDeltaLimits::default(),
        )?;
        match result {
            S3ManifestDeltaOutcome::Complete { base, .. }
                if (previous.is_none() && base == S3ManifestDeltaBase::Genesis)
                    || (previous.is_some()
                        && base == S3ManifestDeltaBase::VerifiedCheckpoint) => {}
            S3ManifestDeltaOutcome::RequiresVerifiedSnapshot(_) => {
                return Err(S3RemoteAnchorError::VerifiedSnapshotRequired);
            }
            S3ManifestDeltaOutcome::NeedOlderManifest { .. } => {
                return Err(S3RemoteAnchorError::IncompleteAncestry);
            }
            _ => return Err(S3RemoteAnchorError::InvalidEvidence),
        }

        let receipt = record_digest(
            &scope.fingerprint, &head_json, &verified.etag, &manifest_bytes,
        );
        if let Some(old) = previous {
            let old_json = old.pointer.encode(&scope.keyspace)?;
            let old_receipt = record_digest(
                &scope.fingerprint, &old_json, &old.etag, &old.manifest_bytes,
            );
            let updated = tx.execute(
                "UPDATE s3_remote_anchor_v1
                 SET head_json=?3,head_etag=?4,manifest_bytes=?5,receipt_digest=?6
                 WHERE profile_id=?1 AND vault_id=?2 AND receipt_digest=?7
                   AND scope_fingerprint=?8",
                params![
                    scope.profile_id, scope.vault_id.to_string(),
                    head_json, verified.etag, manifest_bytes, receipt,
                    old_receipt, scope.fingerprint,
                ],
            )?;
            if updated != 1 {
                return Err(S3RemoteAnchorError::RollbackOrFork);
            }
            tx.commit()?;
            Ok(S3RemoteAnchorOutcome::Advanced)
        } else {
            tx.execute(
                "INSERT INTO s3_remote_anchor_v1
                    (profile_id,vault_id,scope_fingerprint,head_json,head_etag,
                     manifest_bytes,receipt_digest)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    scope.profile_id, scope.vault_id.to_string(), scope.fingerprint,
                    head_json, verified.etag, manifest_bytes, receipt,
                ],
            )?;
            tx.commit()?;
            Ok(S3RemoteAnchorOutcome::Inserted)
        }
    }
}

fn record_digest(fingerprint: &str, head: &[u8], etag: &str, manifest: &[u8]) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(b"akula-remote-observation-anchor-v1");
    for bytes in [fingerprint.as_bytes(), head, etag.as_bytes(), manifest] {
        hash.update(&(bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    hash.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{OperationId, S3ManifestShardRefV1};
    use tempfile::tempdir;

    fn ns(vault: VaultId) -> S3Keyspace {
        S3Keyspace::new("akula/remote", vault).unwrap()
    }
    fn profile() -> S3ConnectionProfile {
        S3ConnectionProfile {
            bucket: "akula-test".into(),
            region: "us-east-1".into(),
            endpoint: Some("http://127.0.0.1:9000".into()),
            force_path_style: true,
        }
    }
    fn scope(vault: VaultId) -> S3RemoteAnchorScope {
        S3RemoteAnchorScope::new("desktop", "minio-instance-001", &profile(), &ns(vault))
            .unwrap()
    }
    fn digest(bytes: &[u8]) -> String {
        blake3::hash(bytes).to_hex().to_string()
    }
    fn frame(
        scope: &S3RemoteAnchorScope,
        generation: u64,
        parent: Option<&S3VerifiedHead>,
        shard_refs: Vec<S3ManifestShardRefV1>,
    ) -> (S3VerifiedHead, S3PinnedRevisionSnapshot, Vec<u8>) {
        let id = OperationId::new();
        let prev = parent.map(|p| (
            p.pointer.manifest_id(), p.pointer.manifest_blake3().to_owned(),
        ));
        let manifest = S3ManifestV1::new(
            scope.keyspace(), id, generation, prev, shard_refs,
        ).unwrap();
        let bytes = manifest.encode(scope.keyspace()).unwrap();
        let pointer = S3HeadPointerV1::new(
            scope.keyspace(), generation, id, &digest(&bytes),
            parent.map(|p| p.pointer.manifest_id()),
        ).unwrap();
        let snapshot_id = format!("{}:{}", id, digest(&bytes));
        let pinned = S3PinnedRevisionSnapshot::for_remote_anchor_test(
            snapshot_id, manifest.total_revisions as usize,
        );
        (S3VerifiedHead { pointer, etag: format!("\"etag-{generation}\""), manifest }, pinned, bytes)
    }

    #[test]
    fn initial_genesis_persists_through_restart_without_document_baseline() {
        let vault = VaultId::new();
        let scope = scope(vault);
        let dir = tempdir().unwrap();
        let path = dir.path().join("remote-anchor.sqlite");
        let (head, pinned, bytes) = frame(&scope, 1, None, vec![]);
        let mut store = S3RemoteAnchorJournal::open(&path).unwrap();
        assert!(store.load(&scope).unwrap().is_none());
        assert_eq!(
            store.record_verified_remote(&scope, &head, &pinned, &[bytes.clone()]).unwrap(),
            S3RemoteAnchorOutcome::Inserted
        );
        assert_eq!(
            store.record_verified_remote(&scope, &head, &pinned, &[]).unwrap(),
            S3RemoteAnchorOutcome::Unchanged
        );
        drop(store);
        let again = S3RemoteAnchorJournal::open(&path).unwrap();
        let loaded = again.load(&scope).unwrap().unwrap();
        assert_eq!(loaded.pointer(), &head.pointer);
        assert_eq!(loaded.etag(), head.etag);
        assert_eq!(loaded.manifest_bytes(), bytes);
    }

    #[test]
    fn two_offline_generations_require_exact_manifest_chain_before_advance() {
        let scope = scope(VaultId::new());
        let dir = tempdir().unwrap();
        let mut store = S3RemoteAnchorJournal::open(&dir.path().join("a.sqlite")).unwrap();
        let (h1, p1, b1) = frame(&scope, 1, None, vec![]);
        let (h2, _, b2) = frame(&scope, 2, Some(&h1), vec![]);
        let (h3, p3, b3) = frame(&scope, 3, Some(&h2), vec![]);
        store.record_verified_remote(&scope, &h1, &p1, &[b1.clone()]).unwrap();
        assert!(matches!(
            store.record_verified_remote(&scope, &h3, &p3, &[b3.clone()]),
            Err(S3RemoteAnchorError::IncompleteAncestry)
        ));
        assert_eq!(store.load(&scope).unwrap().unwrap().pointer(), &h1.pointer);
        assert_eq!(
            store.record_verified_remote(&scope, &h3, &p3, &[b3, b2]).unwrap(),
            S3RemoteAnchorOutcome::Advanced
        );
        assert_eq!(store.load(&scope).unwrap().unwrap().pointer(), &h3.pointer);
    }

    #[test]
    fn local_scope_reuse_with_different_remote_binding_fails_closed() {
        let vault = VaultId::new();
        let scope = scope(vault);
        let dir = tempdir().unwrap();
        let mut store = S3RemoteAnchorJournal::open(&dir.path().join("scope.sqlite")).unwrap();
        let (h1, p1, b1) = frame(&scope, 1, None, vec![]);
        store.record_verified_remote(&scope, &h1, &p1, &[b1]).unwrap();
        let mut wrong_profile = profile();
        wrong_profile.bucket = "another-bucket".into();
        let wrong_bucket = S3RemoteAnchorScope::new(
            "desktop", "minio-instance-001", &wrong_profile, scope.keyspace(),
        ).unwrap();
        assert!(matches!(store.load(&wrong_bucket), Err(S3RemoteAnchorError::ScopeChanged)));
        let wrong_account = S3RemoteAnchorScope::new(
            "desktop", "other-account", &profile(), scope.keyspace(),
        ).unwrap();
        assert!(matches!(store.load(&wrong_account), Err(S3RemoteAnchorError::ScopeChanged)));
        let mut wrong_endpoint = profile();
        wrong_endpoint.endpoint = Some("https://example.net".into());
        let endpoint = S3RemoteAnchorScope::new(
            "desktop", "minio-instance-001", &wrong_endpoint, scope.keyspace(),
        ).unwrap();
        assert!(matches!(store.load(&endpoint), Err(S3RemoteAnchorError::ScopeChanged)));
        let wrong_prefix = S3RemoteAnchorScope::new(
            "desktop", "minio-instance-001", &profile(),
            &S3Keyspace::new("new/prefix", vault).unwrap(),
        ).unwrap();
        assert!(matches!(store.load(&wrong_prefix), Err(S3RemoteAnchorError::ScopeChanged)));
    }

    #[test]
    fn rollback_and_same_generation_fork_cannot_replace_durable_state() {
        let scope = scope(VaultId::new());
        let dir = tempdir().unwrap();
        let mut store = S3RemoteAnchorJournal::open(&dir.path().join("f.sqlite")).unwrap();
        let (h1, p1, b1) = frame(&scope, 1, None, vec![]);
        let (h2, p2, b2) = frame(&scope, 2, Some(&h1), vec![]);
        store.record_verified_remote(&scope, &h1, &p1, &[b1]).unwrap();
        store.record_verified_remote(&scope, &h2, &p2, &[b2]).unwrap();
        assert!(matches!(
            store.record_verified_remote(&scope, &h1, &p1, &[]),
            Err(S3RemoteAnchorError::RollbackOrFork)
        ));
        let (fork, pinned, bytes) = frame(&scope, 2, Some(&h1), vec![]);
        assert!(matches!(
            store.record_verified_remote(&scope, &fork, &pinned, &[bytes]),
            Err(S3RemoteAnchorError::RollbackOrFork)
        ));
        assert_eq!(store.load(&scope).unwrap().unwrap().pointer(), &h2.pointer);
    }

    #[test]
    fn missing_or_corrupt_chain_never_advances_state() {
        let scope = scope(VaultId::new());
        let dir = tempdir().unwrap();
        let mut store = S3RemoteAnchorJournal::open(&dir.path().join("a.sqlite")).unwrap();
        let (h1, p1, b1) = frame(&scope, 1, None, vec![]);
        let (h2, p2, b2) = frame(&scope, 2, Some(&h1), vec![]);
        store.record_verified_remote(&scope, &h1, &p1, &[b1]).unwrap();
        let mut corrupt = b2.clone();
        corrupt.push(b' ');
        assert!(matches!(
            store.record_verified_remote(&scope, &h2, &p2, &[corrupt]),
            Err(S3RemoteAnchorError::IncompleteAncestry)
        ));
        assert!(matches!(
            store.record_verified_remote(&scope, &h2, &p2, &[]),
            Err(S3RemoteAnchorError::IncompleteAncestry)
        ));
        assert_eq!(store.load(&scope).unwrap().unwrap().pointer(), &h1.pointer);
    }

    #[test]
    fn rewritten_etag_or_mismatched_pinned_snapshot_cannot_be_accepted() {
        let scope = scope(VaultId::new());
        let dir = tempdir().unwrap();
        let mut store = S3RemoteAnchorJournal::open(&dir.path().join("a.sqlite")).unwrap();
        let (h1, p1, b1) = frame(&scope, 1, None, vec![]);
        store.record_verified_remote(&scope, &h1, &p1, &[b1]).unwrap();
        let mut etag_change = h1.clone();
        etag_change.etag = "\"other-etag\"".into();
        assert!(matches!(
            store.record_verified_remote(&scope, &etag_change, &p1, &[]),
            Err(S3RemoteAnchorError::VersionRewritten)
        ));
        let other = S3PinnedRevisionSnapshot::for_remote_anchor_test(
            "wrong-manifest-id".into(), 0,
        );
        assert!(matches!(
            store.record_verified_remote(&scope, &h1, &other, &[]),
            Err(S3RemoteAnchorError::InvalidEvidence)
        ));
    }

    #[test]
    fn corrupt_sqlite_row_or_digest_is_rejected_after_restart() {
        let scope = scope(VaultId::new());
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.sqlite");
        let (head, pinned, bytes) = frame(&scope, 1, None, vec![]);
        let mut store = S3RemoteAnchorJournal::open(&path).unwrap();
        store.record_verified_remote(&scope, &head, &pinned, &[bytes]).unwrap();
        store.db.execute(
            "UPDATE s3_remote_anchor_v1 SET manifest_bytes=?1",
            params![b"tampered".to_vec()],
        ).unwrap();
        drop(store);
        let reopened = S3RemoteAnchorJournal::open(&path).unwrap();
        assert!(matches!(reopened.load(&scope), Err(S3RemoteAnchorError::CorruptAnchor)));
    }

    #[test]
    fn second_process_cannot_replay_stale_anchor_after_a_newer_commit() {
        let scope = scope(VaultId::new());
        let dir = tempdir().unwrap();
        let path = dir.path().join("shared.sqlite");
        let mut first = S3RemoteAnchorJournal::open(&path).unwrap();
        let mut second = S3RemoteAnchorJournal::open(&path).unwrap();
        let (h1, p1, b1) = frame(&scope, 1, None, vec![]);
        let (h2, p2, b2) = frame(&scope, 2, Some(&h1), vec![]);
        first.record_verified_remote(&scope, &h1, &p1, &[b1]).unwrap();
        second.record_verified_remote(&scope, &h2, &p2, &[b2]).unwrap();
        assert!(matches!(
            first.record_verified_remote(&scope, &h1, &p1, &[]),
            Err(S3RemoteAnchorError::RollbackOrFork)
        ));
        assert_eq!(first.load(&scope).unwrap().unwrap().pointer(), &h2.pointer);
    }

    #[test]
    fn invalid_profile_labels_and_unsafe_s3_endpoint_are_rejected() {
        let v = VaultId::new();
        for bad in ["", "a b", "vault\n", "@" , &"x".repeat(129)] {
            assert!(matches!(
                S3RemoteAnchorScope::new(bad, "account", &profile(), &ns(v)),
                Err(S3RemoteAnchorError::InvalidScope)
            ));
        }
        let mut bad = profile();
        bad.endpoint = Some("http://malicious.example".into());
        assert!(matches!(
            S3RemoteAnchorScope::new("p", "account", &bad, &ns(v)),
            Err(S3RemoteAnchorError::Transport(S3TransportError::UnsafeEndpoint))
        ));
    }
}
