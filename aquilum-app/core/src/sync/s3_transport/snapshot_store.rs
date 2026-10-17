//! SYNC-05d2: bounded, verified *revision metadata* RemoteStore bridge.
//!
//! S3 is async; RemoteStore is sync. All network IO happens exactly once
//! during async construction on a dedicated Core worker. The resulting object
//! is a private immutable, memory-bounded view of revision JSON, NOT a file
//! mirror, blob transport, live cloud client, or deletion authority.
//!
//! A raw S3 ListObjectsV2 result must never be used as the snapshot source.

use super::{AwsS3ProbeClient, S3HeadError, S3TransportError};
use crate::sync::{
    collect_verified_listing, ProviderCapabilities, RemoteObject, RemotePage,
    RemotePrecondition, RemoteStore, RemoteStoreError, RemoteWriteReceipt,
    RevisionDag, RevisionOperation, RevisionRecord, S3HeadPointerV1,
    S3RevisionRefV1,
};
use std::collections::BTreeMap;

const MAX_SINGLE_REVISION_BYTES: u64 = 64 * 1024;
pub const MAX_S3_REVISION_METADATA_CACHE_BYTES: usize = 32 * 1024 * 1024;
const INTERNAL_PAGE_SIZE: usize = 128;
const MAX_EXTERNAL_PAGE_SIZE: usize = 1_000;

#[derive(Clone, Debug)]
struct StoredVerifiedRevision {
    metadata: RemoteObject,
    json: Vec<u8>,
}

/// A complete, immutable view of **revision JSON objects** rooted in an
/// independently verified S3 manifest and bounded ancestor chain.
///
/// IDs are logical `revisions/<uuid>`, NOT user-relative file paths. The
/// object revision/content hash covers the revision JSON bytes, not the
/// content blob referenced by that revision.
///
/// The store never has an S3 client, filesystem handle, secret, or write
/// capability. No method may advance a confirmed sync baseline.
#[derive(Clone, Debug)]
pub struct S3VerifiedRevisionMetadataStore {
    snapshot_id: String,
    records: BTreeMap<String, StoredVerifiedRevision>,
    total_bytes: usize,
    max_total_bytes: usize,
}

impl S3VerifiedRevisionMetadataStore {
    fn new(snapshot_id: String, max_total_bytes: usize) -> Result<Self, S3HeadError> {
        if snapshot_id.is_empty() || snapshot_id.chars().any(char::is_control)
            || max_total_bytes == 0
            || max_total_bytes > MAX_S3_REVISION_METADATA_CACHE_BYTES
        {
            return Err(S3HeadError::SnapshotBudgetExceeded);
        }
        Ok(Self {
            snapshot_id,
            records: BTreeMap::new(),
            total_bytes: 0,
            max_total_bytes,
        })
    }

    pub fn len(&self) -> usize { self.records.len() }
    pub fn is_empty(&self) -> bool { self.records.is_empty() }
    pub fn total_bytes(&self) -> usize { self.total_bytes }
    pub fn snapshot_id(&self) -> &str { &self.snapshot_id }

    /// Re-check exact immutable bytes and identities even though the network
    /// stage already verified them. Never trust a JSON value merely because
    /// it arrived with a valid snapshot token.
    fn accept(
        &mut self,
        reference: S3RevisionRefV1,
        json: Vec<u8>,
    ) -> Result<(), S3HeadError> {
        if json.len() as u64 > MAX_SINGLE_REVISION_BYTES {
            return Err(S3HeadError::SnapshotBudgetExceeded);
        }
        let next_bytes = self.total_bytes.checked_add(json.len())
            .ok_or(S3HeadError::SnapshotBudgetExceeded)?;
        if next_bytes > self.max_total_bytes {
            return Err(S3HeadError::SnapshotBudgetExceeded);
        }
        let digest = blake3::hash(&json).to_hex().to_string();
        if digest != reference.revision_blake3 {
            return Err(S3HeadError::IntegrityMismatch);
        }
        let record: RevisionRecord = serde_json::from_slice(&json)
            .map_err(|_| S3HeadError::InvalidRevision)?;
        if record.id != reference.revision_id
            || record.document_id != reference.document_id
        {
            return Err(S3HeadError::InvalidRevision);
        }
        match (&record.operation, reference.blob_bytes) {
            (RevisionOperation::Tombstone { .. }, None) => {}
            (RevisionOperation::Content { .. } | RevisionOperation::Rename { .. }, Some(_)) => {}
            _ => return Err(S3HeadError::InvalidRevision),
        }
        let id = format!("revisions/{}", reference.revision_id);
        let metadata = RemoteObject {
            id: id.clone(),
            // Exact content-addressed JSON is an immutable provider version;
            // this is NOT the S3 ETag or an editor/history revision.
            revision: digest.clone(),
            byte_length: json.len() as u64,
            content_hash: Some(digest),
        };
        if self.records.insert(id, StoredVerifiedRevision { metadata, json }).is_some() {
            return Err(S3HeadError::InvalidRevision);
        }
        self.total_bytes = next_bytes;
        Ok(())
    }

    fn validate_complete_dag(&self) -> Result<(), S3HeadError> {
        let mut dag = RevisionDag::new();
        for record in self.records.values() {
            let revision: RevisionRecord = serde_json::from_slice(&record.json)
                .map_err(|_| S3HeadError::InvalidRevision)?;
            dag.ingest(revision)?;
        }
        if dag.pending_len() != 0 || dag.active_len() != self.records.len() {
            return Err(S3HeadError::InvalidManifest);
        }
        Ok(())
    }

    fn cursor_offset(&self, cursor: Option<&str>) -> Result<usize, RemoteStoreError> {
        match cursor {
            None => Ok(0),
            Some(token) => {
                let prefix = format!("{}:", self.snapshot_id);
                let offset = token.strip_prefix(&prefix)
                    .filter(|value| !value.is_empty() && !value.starts_with('0'))
                    .and_then(|value| value.parse::<usize>().ok())
                    .ok_or(RemoteStoreError::InvalidCursor)?;
                if offset == 0 || offset >= self.records.len() {
                    return Err(RemoteStoreError::InvalidCursor);
                }
                Ok(offset)
            }
        }
    }
}

impl RemoteStore for S3VerifiedRevisionMetadataStore {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            conditional_create: false,
            conditional_replace: false,
            conditional_delete: false,
            snapshot_consistency: true,
            version_history: false,
            delta_feed: false,
        }
    }

    fn list_page(
        &self, continuation: Option<&str>, page_limit: usize,
    ) -> Result<RemotePage, RemoteStoreError> {
        if !(1..=MAX_EXTERNAL_PAGE_SIZE).contains(&page_limit) {
            return Err(RemoteStoreError::InvalidCursor);
        }
        let start = self.cursor_offset(continuation)?;
        let end = start.saturating_add(page_limit).min(self.records.len());
        let objects = self.records.values()
            .skip(start).take(end - start)
            .map(|entry| entry.metadata.clone())
            .collect();
        let complete = end == self.records.len();
        Ok(RemotePage {
            objects,
            next_cursor: if complete {
                None
            } else {
                Some(format!("{}:{end}", self.snapshot_id))
            },
            complete,
            snapshot_id: self.snapshot_id.clone(),
        })
    }

    fn read(&self, object_id: &str) -> Result<Vec<u8>, RemoteStoreError> {
        self.records.get(object_id)
            .map(|entry| entry.json.clone())
            .ok_or(RemoteStoreError::NotFound)
    }

    fn read_bounded(
        &self, object_id: &str, max_bytes: u64,
    ) -> Result<Vec<u8>, RemoteStoreError> {
        let item = self.records.get(object_id).ok_or(RemoteStoreError::NotFound)?;
        if item.json.len() as u64 > max_bytes {
            return Err(RemoteStoreError::ObjectTooLarge);
        }
        Ok(item.json.clone())
    }

    fn head(&self, object_id: &str) -> Result<RemoteObject, RemoteStoreError> {
        self.records.get(object_id)
            .map(|entry| entry.metadata.clone())
            .ok_or(RemoteStoreError::NotFound)
    }

    fn put(
        &mut self, _object_id: &str, _bytes: &[u8],
        _precondition: &RemotePrecondition,
    ) -> Result<RemoteWriteReceipt, RemoteStoreError> {
        Err(RemoteStoreError::PreconditionUnsupported)
    }

    fn delete(
        &mut self, _object_id: &str, _precondition: &RemotePrecondition,
    ) -> Result<(), RemoteStoreError> {
        Err(RemoteStoreError::PreconditionUnsupported)
    }
}

impl AwsS3ProbeClient {
    /// Prepare a synchronous RemoteStore OFF the UI thread. Only after
    /// S3 root, all immutable manifest shards, full revision DAG, all blobs,
    /// and bounded ancestor-chain have passed verification, fetch the exact
    /// immutable revision JSON again into a bounded in-memory snapshot.
    ///
    /// None means no S3 root exists: the caller MUST handle bootstrap
    /// separately, never interpret None as a verified empty remote vault.
    /// Changes to the root during materialization produce Conflict/Hold.
    pub async fn materialize_verified_revision_metadata_store(
        &self, max_total_bytes: usize,
    ) -> Result<Option<S3VerifiedRevisionMetadataStore>, S3HeadError> {
        if max_total_bytes == 0 || max_total_bytes > MAX_S3_REVISION_METADATA_CACHE_BYTES {
            return Err(S3HeadError::SnapshotBudgetExceeded);
        }
        let Some(snapshot) = self.pin_revision_snapshot().await? else {
            return Ok(None);
        };
        let mut view = S3VerifiedRevisionMetadataStore::new(
            snapshot.snapshot_id.clone(), max_total_bytes,
        )?;
        let mut cursor = None;
        loop {
            let page = snapshot.page(cursor.as_deref(), INTERNAL_PAGE_SIZE)?;
            for reference in page.revisions {
                let key = self.keyspace.immutable_revision_key(reference.revision_id);
                let revision = self.get_bounded(&key, MAX_SINGLE_REVISION_BYTES).await?;
                view.accept(reference, revision.bytes)?;
            }
            if page.complete { break; }
            cursor = Some(page.next_cursor.ok_or(S3HeadError::InvalidCursor)?);
        }
        view.validate_complete_dag()?;

        // A remote writer may advance refs/current.json during preparation.
        // A previously valid immutable snapshot remains available remotely,
        // but it must not be mislabeled the current inventory for re-planning.
        // Read only the small root (HEAD/GET If-Match/HEAD); don't repeat the
        // expensive DAG download merely to test root identity.
        let current = match self.get_bounded(&self.keyspace.current_head_key(), 4096).await {
            Ok(value) => value,
            Err(S3TransportError::NotFound) => return Err(S3HeadError::Conflict),
            Err(error) => return Err(error.into()),
        };
        let pointer = S3HeadPointerV1::decode(&current.bytes, &self.keyspace)
            .map_err(|_| S3HeadError::InvalidManifest)?;
        let current_id = format!("{}:{}", pointer.manifest_id(), pointer.manifest_blake3());
        if current_id != view.snapshot_id {
            return Err(S3HeadError::Conflict);
        }
        Ok(Some(view))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{DocumentId, RevisionId, SyncRelativePath};

    fn fixture(
        document: DocumentId,
        parents: Vec<RevisionId>,
        content: &[u8],
    ) -> (S3RevisionRefV1, Vec<u8>) {
        let revision = RevisionRecord {
            id: RevisionId::new(),
            document_id: document,
            parents,
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse("notes/test.md").unwrap(),
                blob_hash: blake3::hash(content).to_hex().to_string(),
            },
        };
        let bytes = serde_json::to_vec(&revision).unwrap();
        (S3RevisionRefV1 {
            revision_id: revision.id,
            document_id: document,
            revision_blake3: blake3::hash(&bytes).to_hex().to_string(),
            blob_bytes: Some(content.len() as u64),
        }, bytes)
    }

    fn view_three() -> S3VerifiedRevisionMetadataStore {
        let document = DocumentId::new();
        let (root, a) = fixture(document, vec![], b"root");
        let (left, b) = fixture(document, vec![root.revision_id], b"alice");
        let (right, c) = fixture(document, vec![root.revision_id], b"bob");
        let mut store = S3VerifiedRevisionMetadataStore::new(
            "manifest-identity:verified-hash".into(), 1_048_576,
        ).unwrap();
        // Unordered arrivals are okay; closure is checked at the end.
        store.accept(left, b).unwrap();
        store.accept(right, c).unwrap();
        store.accept(root, a).unwrap();
        store.validate_complete_dag().unwrap();
        store
    }

    #[test]
    fn provider_collects_complete_pinned_pages_and_never_mutates() {
        let mut store = view_three();
        assert_eq!(store.len(), 3);
        assert!(store.total_bytes() > 0);
        let listing = collect_verified_listing(&store, 1, 3).unwrap();
        assert_eq!(listing.objects.len(), 3);
        assert_eq!(listing.snapshot_id, store.snapshot_id());
        assert!(listing.objects.iter().all(|o| o.id.starts_with("revisions/")
            && o.content_hash.as_deref() == Some(o.revision.as_str())));
        assert!(store.capabilities().snapshot_consistency);
        assert!(!store.capabilities().supports_guarded_mutation());
        let entry = &listing.objects[0];
        assert_eq!(store.head(&entry.id).unwrap(), *entry);
        assert_eq!(store.read(&entry.id).unwrap().len() as u64, entry.byte_length);
        assert_eq!(store.read_bounded(&entry.id, entry.byte_length - 1),
            Err(RemoteStoreError::ObjectTooLarge));
        assert_eq!(store.read_bounded(&entry.id, entry.byte_length).unwrap(),
            store.read(&entry.id).unwrap());
        assert_eq!(store.put(&entry.id, b"modified", &RemotePrecondition::Absent),
            Err(RemoteStoreError::PreconditionUnsupported));
        assert_eq!(store.delete(&entry.id, &RemotePrecondition::Absent),
            Err(RemoteStoreError::PreconditionUnsupported));
    }

    #[test]
    fn forged_cross_snapshot_or_ambiguous_cursors_fail_closed() {
        let store = view_three();
        let first = store.list_page(None, 2).unwrap();
        assert!(!first.complete);
        assert_eq!(first.objects.len(), 2);
        assert_eq!(first.snapshot_id, store.snapshot_id);
        let tail = store.list_page(first.next_cursor.as_deref(), 2).unwrap();
        assert!(tail.complete);
        assert_eq!(tail.objects.len(), 1);
        let other = S3VerifiedRevisionMetadataStore::new("other:root".into(), 1024).unwrap();
        assert_eq!(other.list_page(first.next_cursor.as_deref(), 2),
            Err(RemoteStoreError::InvalidCursor));
        for cursor in [
            "other:1", "manifest-identity:verified-hash:0",
            "manifest-identity:verified-hash:03",
            "manifest-identity:verified-hash:3",
            "manifest-identity:verified-hash:-2", "manifest-identity:verified-hash:nan",
        ] {
            assert_eq!(store.list_page(Some(cursor), 2),
                Err(RemoteStoreError::InvalidCursor));
        }
        assert_eq!(store.list_page(None, 0), Err(RemoteStoreError::InvalidCursor));
        assert_eq!(store.list_page(None, 1001), Err(RemoteStoreError::InvalidCursor));
        assert_eq!(collect_verified_listing(&store, 1, 2),
            Err(RemoteStoreError::PageLimitExceeded));
        assert_eq!(store.head("blobs/not-a-revision"),
            Err(RemoteStoreError::NotFound));
    }

    #[test]
    fn rejects_invalid_bytes_and_missing_parent_without_treating_as_empty() {
        let doc = DocumentId::new();
        let (root, bytes) = fixture(doc, vec![], b"valid");
        let mut store = S3VerifiedRevisionMetadataStore::new("root:valid".into(), 1024).unwrap();
        let mut changed = bytes.clone();
        changed.push(b' ');
        assert!(matches!(store.accept(root.clone(), changed),
            Err(S3HeadError::IntegrityMismatch)));
        let mut wrong = root.clone();
        wrong.document_id = DocumentId::new();
        assert!(matches!(store.accept(wrong, bytes.clone()),
            Err(S3HeadError::InvalidRevision)));
        let missing = fixture(doc, vec![RevisionId::new()], b"child");
        store.accept(missing.0, missing.1).unwrap();
        assert!(matches!(store.validate_complete_dag(),
            Err(S3HeadError::InvalidManifest)));
        assert!(matches!(store.accept(root.clone(), bytes),
            Err(S3HeadError::SnapshotBudgetExceeded)) == false);
        assert!(matches!(S3VerifiedRevisionMetadataStore::new("root".into(), 0),
            Err(S3HeadError::SnapshotBudgetExceeded)));
        assert!(matches!(S3VerifiedRevisionMetadataStore::new("root".into(),
            MAX_S3_REVISION_METADATA_CACHE_BYTES + 1),
            Err(S3HeadError::SnapshotBudgetExceeded)));
    }

    #[test]
    fn budget_is_checked_before_cache_grows() {
        let doc = DocumentId::new();
        let (reference, bytes) = fixture(doc, vec![], b"safe");
        let budget = bytes.len() - 1;
        let mut store = S3VerifiedRevisionMetadataStore::new("id:digest".into(), budget).unwrap();
        assert!(matches!(store.accept(reference, bytes),
            Err(S3HeadError::SnapshotBudgetExceeded)));
        assert_eq!(store.len(), 0);
        assert_eq!(store.total_bytes(), 0);
    }
}
