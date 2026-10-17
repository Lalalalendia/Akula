//! SYNC-05d2b: fail-closed preview after a rejected/uncertain S3 head CAS.
//!
//! A preview is NOT a remote write operation, a local File Gate action,
//! evidence of an acknowledged write, or authority to drop either DAG head.
//! Only complete vault-bound inventories and immutable, BLAKE3-verified
//! revision JSON contribute to the unified graph. No automatic CAS retry.

use super::{
    AwsS3ProbeClient, S3HeadError, S3TransportError,
    S3VerifiedHead, S3VerifiedRevisionMetadataStore,
};
use crate::sync::{
    plan_revision_exchange, DagError, DeltaPlanError, DocumentId, RevisionDag,
    RevisionExchangePlan, RevisionId, RevisionInventory, RevisionRecord,
    RemoteStore, RemoteStoreError, S3HeadPointerV1, S3Keyspace,
};
use std::collections::{BTreeMap, BTreeSet};

const MAX_REPLAN_ANCESTORS: usize = 1_024;
const MAX_REVISION_JSON: u64 = 64 * 1024;

#[derive(Debug)]
pub enum S3ReplanError {
    Remote(S3HeadError),
    Provider(RemoteStoreError),
    Delta(DeltaPlanError),
    Dag(DagError),
    InvalidRemoteRevision,
    PreviousHeadNotVerified,
    MissingCurrentHead,
    UnchangedCurrentHead,
    DisconnectedHeadHistory,
    AncestorLimitExceeded,
    HeadChangedDuringPreview,
}
impl From<S3HeadError> for S3ReplanError {
    fn from(value: S3HeadError) -> Self { Self::Remote(value) }
}
impl From<RemoteStoreError> for S3ReplanError {
    fn from(value: RemoteStoreError) -> Self { Self::Provider(value) }
}
impl From<DeltaPlanError> for S3ReplanError {
    fn from(value: DeltaPlanError) -> Self { Self::Delta(value) }
}
impl From<DagError> for S3ReplanError {
    fn from(value: DagError) -> Self { Self::Dag(value) }
}

/// Read-only HAVE/WANT metadata. Conflicting heads remain explicit and
/// every candidate blob is only a transfer *hint*, not proof of availability.
/// No deletion, File Gate write, journal confirmation, CAS or auto-merge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3CasReplanPreview {
    pub local_snapshot_id: String,
    pub remote_snapshot_id: String,
    pub exchange: RevisionExchangePlan,
    pub conflicting_heads: BTreeMap<DocumentId, BTreeSet<RevisionId>>,
}

/// Compare a verified local scan with a previously materialized and complete
/// S3 revision metadata store, rebuilding the *union* DAG. This never uses
/// raw S3 object listings, remote ETags as content hashes, or file mtimes.
///
/// A stale local inventory, foreign vault, missing parent, collision or
/// incomplete S3 view is an error, not an empty result/deletion instruction.
pub fn plan_verified_s3_revision_replay(
    local_dag: &RevisionDag,
    local: &RevisionInventory,
    remote: &S3VerifiedRevisionMetadataStore,
) -> Result<S3CasReplanPreview, S3ReplanError> {
    let remote_inventory = remote.verified_revision_inventory()?;
    if local.vault_id != remote_inventory.vault_id {
        return Err(S3ReplanError::Delta(DeltaPlanError::VaultMismatch));
    }
    // The provider-neutral planner will independently check that both sets
    // are complete, snapshot-labelled and transitively parent-closed.
    let mut union = RevisionDag::new();
    let mut documents = BTreeSet::new();
    for revision_id in &local.revisions {
        let record = local_dag.get(*revision_id)
            .ok_or(DeltaPlanError::UnknownRevision(*revision_id))?;
        documents.insert(record.document_id);
        union.ingest(record.clone())?;
    }
    for revision_id in &remote_inventory.revisions {
        let key = format!("revisions/{revision_id}");
        let metadata = remote.head(&key)?;
        let json = remote.read_bounded(&key, MAX_REVISION_JSON)?;
        let digest = blake3::hash(&json).to_hex().to_string();
        if metadata.id != key
            || metadata.content_hash.as_deref() != Some(digest.as_str())
            || metadata.revision != digest
            || metadata.byte_length != json.len() as u64
        { return Err(S3ReplanError::InvalidRemoteRevision); }
        let record: RevisionRecord = serde_json::from_slice(&json)
            .map_err(|_| S3ReplanError::InvalidRemoteRevision)?;
        if record.id != *revision_id {
            return Err(S3ReplanError::InvalidRemoteRevision);
        }
        documents.insert(record.document_id);
        union.ingest(record)?;
    }
    if union.pending_len() != 0
        || union.active_len() != local.revisions.union(&remote_inventory.revisions).count()
    {
        return Err(S3ReplanError::InvalidRemoteRevision);
    }
    let exchange = plan_revision_exchange(&union, local, &remote_inventory)?;
    let mut conflicting_heads = BTreeMap::new();
    for doc in documents {
        let heads = union.heads(doc);
        if heads.len() > 1 {
            conflicting_heads.insert(doc, heads);
        }
    }
    Ok(S3CasReplanPreview {
        local_snapshot_id: local.snapshot_id.clone(),
        remote_snapshot_id: remote_inventory.snapshot_id,
        exchange,
        conflicting_heads,
    })
}

/// A caller-supplied S3VerifiedHead is not sufficient evidence unless its
/// in-memory pointer and manifest agree on the *same* immutable generation,
/// digest and predecessor. read_verified_manifest then independently checks
/// those bytes and referenced objects at the provider.
fn validate_supplied_previous_head(
    previous: &S3VerifiedHead,
    keyspace: &S3Keyspace,
) -> Result<(), S3ReplanError> {
    let claimed_hash = previous.manifest.digest(keyspace)
        .map_err(|_| S3ReplanError::PreviousHeadNotVerified)?;
    if previous.etag.is_empty()
        || previous.etag.chars().any(char::is_control)
        || previous.pointer.encode(keyspace).is_err()
        || previous.manifest.manifest_id != previous.pointer.manifest_id()
        || previous.manifest.generation != previous.pointer.generation()
        || previous.manifest.previous_manifest_id != previous.pointer.previous_manifest_id()
        || claimed_hash != previous.pointer.manifest_blake3()
    {
        return Err(S3ReplanError::PreviousHeadNotVerified);
    }
    Ok(())
}

impl AwsS3ProbeClient {
    /// Refresh a root after rejected CAS with no blind retry:
    ///
    /// 1. Revalidate the previous immutable root, rejecting spoofed callers.
    /// 2. Verify the new S3 head advanced along the *same* ancestor chain.
    /// 3. Materialize the current complete DAG, bounded by metadata budget.
    /// 4. Build a read-only preview of both clients' revisions/heads.
    /// 5. Check the mutable S3 head still matches the same ETag + pointer.
    ///
    /// If the remote root did not advance, the failed PUT may be ambiguous:
    /// Return Hold instead of silently creating a fresh operation. The caller
    /// must independently ensure that the local inventory still corresponds
    /// to its pinned scan before any later explicit publication.
    pub async fn preview_replan_after_s3_cas_conflict(
        &self,
        previous: &S3VerifiedHead,
        local_dag: &RevisionDag,
        local_inventory: &RevisionInventory,
        max_metadata_bytes: usize,
    ) -> Result<S3CasReplanPreview, S3ReplanError> {
        validate_supplied_previous_head(previous, &self.keyspace)?;
        let checked_previous = self.read_verified_manifest(
            previous.pointer.manifest_id(), previous.pointer.manifest_blake3(),
        ).await?;
        if checked_previous != previous.manifest
            || checked_previous.generation != previous.pointer.generation()
        {
            return Err(S3ReplanError::PreviousHeadNotVerified);
        }
        let now = self.read_verified_head().await?
            .ok_or(S3ReplanError::MissingCurrentHead)?;
        if now.pointer == previous.pointer {
            return Err(S3ReplanError::UnchangedCurrentHead);
        }
        if now.pointer.generation() <= previous.pointer.generation() {
            return Err(S3ReplanError::DisconnectedHeadHistory);
        }

        // Explicit ancestry check: a replacement S3 root that is not a
        // descendant of the caller's previously confirmed generation cannot
        // silently become the new baseline, even with a valid standalone DAG.
        let mut cursor = now.manifest.clone();
        let mut found_old_root = false;
        for _ in 0..=MAX_REPLAN_ANCESTORS {
            if cursor.manifest_id == previous.pointer.manifest_id() {
                if cursor.generation != previous.pointer.generation()
                    || cursor.digest(&self.keyspace)
                        .map_err(S3HeadError::from)? != previous.pointer.manifest_blake3()
                {
                    return Err(S3ReplanError::DisconnectedHeadHistory);
                }
                found_old_root = true;
                break;
            }
            let (Some(parent_id), Some(parent_hash)) = (
                cursor.previous_manifest_id, cursor.previous_manifest_blake3.as_deref(),
            ) else {
                return Err(S3ReplanError::DisconnectedHeadHistory);
            };
            let parent = self.read_verified_manifest(parent_id, parent_hash).await?;
            if !cursor.is_successor_of(&parent, parent_hash) {
                return Err(S3ReplanError::DisconnectedHeadHistory);
            }
            cursor = parent;
        }
        if !found_old_root {
            return Err(S3ReplanError::AncestorLimitExceeded);
        }
        let verified = self.materialize_verified_revision_metadata_store(
            max_metadata_bytes,
        ).await?.ok_or(S3ReplanError::MissingCurrentHead)?;
        let current_snapshot_id = format!(
            "{}:{}", now.pointer.manifest_id(), now.pointer.manifest_blake3(),
        );
        if verified.snapshot_id() != current_snapshot_id {
            return Err(S3ReplanError::HeadChangedDuringPreview);
        }
        let preview = plan_verified_s3_revision_replay(
            local_dag, local_inventory, &verified,
        )?;
        // Re-read only the small root; the verified immutable objects need
        // not be fetched again. Both pointer bytes and ETag must still match.
        let root = match self.get_bounded(&self.keyspace.current_head_key(), 4096).await {
            Ok(value) => value,
            Err(S3TransportError::NotFound) => {
                return Err(S3ReplanError::HeadChangedDuringPreview);
            }
            Err(error) => return Err(S3ReplanError::Remote(error.into())),
        };
        let latest = S3HeadPointerV1::decode(&root.bytes, &self.keyspace)
            .map_err(|_| S3ReplanError::HeadChangedDuringPreview)?;
        if latest != now.pointer || root.object.etag != now.etag {
            return Err(S3ReplanError::HeadChangedDuringPreview);
        }
        Ok(preview)
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use crate::sync::{OperationId, S3ManifestV1, VaultId};

    #[test]
    fn supplied_previous_head_requires_consistent_manifest_and_parent_link() {
        let keyspace = S3Keyspace::new("akula-sync/replan", VaultId::new()).unwrap();
        let parent = S3ManifestV1::new(
            &keyspace, OperationId::new(), 1, None, vec![],
        ).unwrap();
        let parent_hash = parent.digest(&keyspace).unwrap();
        let child = S3ManifestV1::new(
            &keyspace, OperationId::new(), 2,
            Some((parent.manifest_id, parent_hash)), vec![],
        ).unwrap();
        let head = S3VerifiedHead {
            pointer: S3HeadPointerV1::new(
                &keyspace, child.generation, child.manifest_id,
                &child.digest(&keyspace).unwrap(), child.previous_manifest_id,
            ).unwrap(),
            etag: "\"observed-s3-etag\"".into(),
            manifest: child,
        };
        assert!(validate_supplied_previous_head(&head, &keyspace).is_ok());

        // This pointer is independently well-formed, but lies about the
        // predecessor of the exact hashed manifest. The old implementation
        // verified manifest bytes without rejecting that contradictory pair.
        let mut forged_parent = head.clone();
        forged_parent.pointer = S3HeadPointerV1::new(
            &keyspace, head.pointer.generation(),
            head.pointer.manifest_id(), head.pointer.manifest_blake3(),
            Some(OperationId::new()),
        ).unwrap();
        assert!(matches!(
            validate_supplied_previous_head(&forged_parent, &keyspace),
            Err(S3ReplanError::PreviousHeadNotVerified)
        ));

        let mut forged_generation = head.clone();
        forged_generation.pointer = S3HeadPointerV1::new(
            &keyspace, 3, head.pointer.manifest_id(),
            head.pointer.manifest_blake3(), head.pointer.previous_manifest_id(),
        ).unwrap();
        assert!(matches!(
            validate_supplied_previous_head(&forged_generation, &keyspace),
            Err(S3ReplanError::PreviousHeadNotVerified)
        ));

        let mut missing_validator = head.clone();
        missing_validator.etag.clear();
        assert!(matches!(
            validate_supplied_previous_head(&missing_validator, &keyspace),
            Err(S3ReplanError::PreviousHeadNotVerified)
        ));
        assert!(matches!(
            validate_supplied_previous_head(
                &head, &S3Keyspace::new("akula-sync/replan", VaultId::new()).unwrap(),
            ),
            Err(S3ReplanError::PreviousHeadNotVerified)
        ));
    }
}
