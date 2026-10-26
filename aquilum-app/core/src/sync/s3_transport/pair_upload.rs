//! User-approved NEW-file sender. No remote deletion, existing-file update,
//! host-file mutation, confirmed baseline or transport credential discovery.
//! Every remote root change is a journaled provider CAS from a pinned origin.

use std::path::Path;
use super::{AwsS3ProbeClient, S3HeadError, S3HeadJournal, S3HeadPublication,
    S3PutCondition, S3TransportError};
use crate::app_core::Core;
use crate::sync::{
    DocumentId, OperationId, RegisteredReadAdmission, RevisionId,
    RevisionOperation, RevisionRecord, S3HeadPointerV1, S3Keyspace,
    S3ManifestError, S3ManifestShardV1, S3ManifestV1, S3RevisionRefV1,
    SyncRelativePath, VaultPortError,
};
use crate::sync::s3_pair_bootstrap::{
    verify_invited_genesis, S3PairBootstrapError,
};

const MAX_NEW_FILE_BYTES: u64 = 1_048_576;
const MAX_REVISION_METADATA: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum S3PairUploadError {
    Local(VaultPortError),
    Head(S3HeadError),
    Transport(S3TransportError),
    Manifest(S3ManifestError),
    Trust(S3PairBootstrapError),
    ForeignScope,
    ExistingPath,
    AlreadyPublishedSame,
    ChangedLocalFile,
    DataMismatch,
    Serialization,
    TooLarge,
}
impl From<VaultPortError> for S3PairUploadError {
    fn from(e: VaultPortError) -> Self { Self::Local(e) }
}
impl From<S3HeadError> for S3PairUploadError {
    fn from(e: S3HeadError) -> Self { Self::Head(e) }
}
impl From<S3TransportError> for S3PairUploadError {
    fn from(e: S3TransportError) -> Self { Self::Transport(e) }
}
impl From<S3ManifestError> for S3PairUploadError {
    fn from(e: S3ManifestError) -> Self { Self::Manifest(e) }
}
impl From<S3PairBootstrapError> for S3PairUploadError {
    fn from(e: S3PairBootstrapError) -> Self { Self::Trust(e) }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3PairRemotePublication {
    pub document_id: DocumentId,
    pub revision_id: RevisionId,
    pub content_hash: String,
    /// S3 immutable+CAS verified. NOT a durable local receipt or per-note ACK.
    pub root_generation: u64,
}

fn touches_path(record: &RevisionRecord, wanted: &SyncRelativePath) -> bool {
    let wanted = wanted.collision_key();
    match &record.operation {
        RevisionOperation::Content { path, .. } |
        RevisionOperation::Tombstone { path } => path.collision_key() == wanted,
        RevisionOperation::Rename { from, to, .. } =>
            from.collision_key() == wanted || to.collision_key() == wanted,
    }
}

/// Safe only for a registered CLOSED local note. The existing Core preflight
/// rejects a dirty/open document and checks the pinned local pathname.
impl AwsS3ProbeClient {
    pub(crate) async fn publish_new_registered_s3_file(
        &self,
        core: &Core,
        registered_root: &Path,
        relative: SyncRelativePath,
        expected_keyspace: &S3Keyspace,
        invited_genesis_fingerprint: &str,
        cloud_journal: &mut S3HeadJournal,
    ) -> Result<S3PairRemotePublication, S3PairUploadError> {
        if self.bound_keyspace() != expected_keyspace {
            return Err(S3PairUploadError::ForeignScope);
        }
        // This is an explicit first trusted device or imported second device:
        // never capture from a random on-disk pathname or unverified S3 root.
        let admission = RegisteredReadAdmission::open(core, registered_root)?;
        let first = admission.read_closed_preflight(&relative, MAX_NEW_FILE_BYTES)?;
        if first.bytes.len() as u64 > MAX_NEW_FILE_BYTES {
            return Err(S3PairUploadError::TooLarge);
        }
        let current = verify_invited_genesis(
            self, expected_keyspace, invited_genesis_fingerprint,
        ).await?;
        let verified = self.materialize_verified_revision_metadata_store(
            MAX_REVISION_METADATA,
        ).await?.ok_or(S3PairUploadError::ForeignScope)?;
        let mut references = Vec::new();
        let mut previously_published = false;
        for (reference, json) in verified.verified_entries() {
            let known: RevisionRecord = serde_json::from_slice(json)
                .map_err(|_| S3PairUploadError::Serialization)?;
            if touches_path(&known, &relative) {
                // Existing exact content is idempotent. A case alias,
                // conflicting revision, tombstone, rename or edited content
                // must HOLD rather than silently claim the file was sent.
                let same = matches!(&known.operation,
                    RevisionOperation::Content { path, blob_hash }
                        if path == &relative && blob_hash == &first.content_hash
                            && known.parents.is_empty());
                if previously_published || !same {
                    return Err(S3PairUploadError::ExistingPath);
                }
                previously_published = true;
                continue;
            }
            references.push(reference.clone());
        }
        if previously_published {
            let second = admission.read_closed_preflight(&relative, MAX_NEW_FILE_BYTES)?;
            if second.bytes != first.bytes || second.content_hash != first.content_hash {
                return Err(S3PairUploadError::ChangedLocalFile);
            }
            return Err(S3PairUploadError::AlreadyPublishedSame);
        }
        // Every remote immutable blob name is a SHA-256-style-looking BLAKE3
        // content address with exact readback, never a user-controlled path.
        let hash = first.content_hash.clone();
        let blob_key = expected_keyspace.immutable_blob_key(&hash)
            .map_err(|_| S3PairUploadError::DataMismatch)?;
        let blob_write = self.put_conditionally(
            &blob_key, &first.bytes, S3PutCondition::IfAbsent,
        ).await;
        match blob_write {
            Ok(_) | Err(S3TransportError::CommitUncertain)
                | Err(S3TransportError::PreconditionFailed) => {}
            Err(error) => return Err(error.into()),
        }
        let published_blob = self.get_bounded(&blob_key, MAX_NEW_FILE_BYTES).await?;
        if published_blob.bytes != first.bytes || published_blob.blake3 != hash {
            return Err(S3PairUploadError::DataMismatch);
        }

        let record = RevisionRecord {
            id: RevisionId::new(),
            document_id: DocumentId::new(),
            parents: vec![],
            operation: RevisionOperation::Content {
                path: relative.clone(),
                blob_hash: hash.clone(),
            },
        };
        let revision_json = serde_json::to_vec(&record)
            .map_err(|_| S3PairUploadError::Serialization)?;
        let revision_key = expected_keyspace.immutable_revision_key(record.id);
        match self.put_conditionally(
            &revision_key, &revision_json, S3PutCondition::IfAbsent,
        ).await {
            Ok(_) | Err(S3TransportError::CommitUncertain)
                | Err(S3TransportError::PreconditionFailed) => {}
            Err(error) => return Err(error.into()),
        }
        let stored = self.get_bounded(&revision_key, 64 * 1024).await?;
        if stored.bytes != revision_json {
            return Err(S3PairUploadError::DataMismatch);
        }
        references.push(S3RevisionRefV1 {
            revision_id: record.id,
            document_id: record.document_id,
            revision_blake3: blake3::hash(&revision_json).to_hex().to_string(),
            blob_bytes: Some(first.bytes.len() as u64),
        });
        references.sort_by_key(|entry| entry.revision_id);
        if references.windows(2).any(|pair| pair[0].revision_id == pair[1].revision_id) {
            return Err(S3PairUploadError::DataMismatch);
        }

        let mut shards = Vec::new();
        for chunk in references.chunks(128) {
            let shard_id = OperationId::new();
            let shard = S3ManifestShardV1::new(
                expected_keyspace, shard_id, chunk.to_vec(),
            )?;
            self.store_manifest_object(shard_id, &shard.encode(expected_keyspace)?)
                .await?;
            shards.push(shard.reference(expected_keyspace)?);
        }
        let manifest_id = OperationId::new();
        let old_digest = current.manifest.digest(expected_keyspace)?;
        let generation = current.manifest.generation.checked_add(1)
            .ok_or(S3PairUploadError::DataMismatch)?;
        let manifest = S3ManifestV1::new(
            expected_keyspace, manifest_id, generation,
            Some((current.manifest.manifest_id, old_digest)), shards,
        )?;
        let manifest_hash = self.store_manifest_object(
            manifest_id, &manifest.encode(expected_keyspace)?,
        ).await?;

        // A locally changed source cannot be advertised as a stable version.
        // This second preflight is conservative (not a cross-process lease).
        let fresh = admission.read_closed_preflight(&relative, MAX_NEW_FILE_BYTES)?;
        if fresh.content_hash != first.content_hash || fresh.bytes != first.bytes {
            return Err(S3PairUploadError::ChangedLocalFile);
        }
        let pointer = S3HeadPointerV1::new(
            expected_keyspace, generation, manifest_id,
            &manifest_hash, Some(current.manifest.manifest_id),
        ).map_err(|_| S3PairUploadError::DataMismatch)?;
        // publish_head_cas journals BEFORE touching the mutable root; no
        // caller may retry this operation ID blindly after an uncertain ACK.
        match self.publish_head_cas(
            cloud_journal, manifest_id, pointer, Some(&current),
        ).await? {
            S3HeadPublication::Published | S3HeadPublication::Recovered => {}
        }
        Ok(S3PairRemotePublication {
            document_id: record.document_id,
            revision_id: record.id,
            content_hash: hash,
            root_generation: generation,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn create_only_sender_holds_all_foreign_path_aliases() {
        let wanted = SyncRelativePath::parse("Notes/Hello.md").unwrap();
        let other = SyncRelativePath::parse("notes/hello.md").unwrap();
        let doc = DocumentId::new();
        let record = RevisionRecord {
            id: RevisionId::new(), document_id: doc, parents: vec![],
            operation: RevisionOperation::Content {
                path: other, blob_hash: "0".repeat(64),
            },
        };
        assert!(touches_path(&record, &wanted));
    }
}
