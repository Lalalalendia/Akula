//! Provider-neutral remote contract. The core planner must never infer deletion from
//! a failed, incomplete, stale or permission-filtered listing.
//!
//! This interface is synchronous on purpose: PR 04 runs it on a dedicated worker,
//! never on the native UI thread. Providers can use their own I/O clients internally.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderCapabilities {
    /// Atomic create-if-absent of an object at the provider.
    pub conditional_create: bool,
    /// Atomic replacement guarded by an opaque provider revision.
    pub conditional_replace: bool,
    /// Atomic deletion guarded by an opaque provider revision.
    pub conditional_delete: bool,
    /// A listing represents a stable remote snapshot, not just individual pages.
    pub snapshot_consistency: bool,
    /// Older provider revisions are retrievable after a write.
    pub version_history: bool,
    /// Complete resumable changes feed, where supported.
    pub delta_feed: bool,
}

impl ProviderCapabilities {
    pub const fn supports_guarded_mutation(self) -> bool {
        self.conditional_create && self.conditional_replace && self.conditional_delete
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteObject {
    /// Opaque provider identity. Never interpret this as a local filesystem path.
    pub id: String,
    /// Opaque provider revision/validator. Not a content checksum.
    pub revision: String,
    pub byte_length: u64,
    /// BLAKE3 of verified bytes when supplied by the sync layer, not S3 ETag.
    pub content_hash: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemotePage {
    pub objects: Vec<RemoteObject>,
    /// Must be followed until exhausted. A missing cursor alone is not proof of
    /// completeness unless `complete` is also true.
    pub next_cursor: Option<String>,
    pub complete: bool,
    /// Opaque snapshot token for replay / detecting a changed listing.
    pub snapshot_id: String,
}

impl RemotePage {
    pub fn is_final_verified_page(&self) -> bool {
        self.complete && self.next_cursor.is_none()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemotePrecondition {
    /// No object with this identity exists.
    Absent,
    /// The existing object has exactly this opaque provider revision.
    Matches(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteWriteReceipt {
    /// A confirmed provider revision. Callers must still verify remote bytes
    /// before advancing their durable baseline.
    pub revision: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoteStoreError {
    NotFound,
    PermissionDenied,
    Unavailable,
    InvalidCursor,
    ListingChanged,
    IncompleteListing,
    UnverifiableListing,
    DuplicateRemoteObject,
    InvalidListing,
    PageLimitExceeded,
    PreconditionFailed,
    PreconditionUnsupported,
    /// Write may have happened despite a lost/failed response: MUST read back.
    CommitUncertain,
    IntegrityMismatch,
}

/// The provider is an exchange service, not a deletion authority. In particular,
/// all mutations require explicit preconditions; backends that cannot provide an
/// atomic guard must reject them, not silently degrade to last-write-wins.
/// A fully collected, internally consistent remote object listing. Its presence
/// is not proof that a deletion is authorized: the planner still checks the
/// confirmed baseline, explicit tombstones and device policies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSnapshot {
    pub snapshot_id: String,
    pub objects: Vec<RemoteObject>,
}

/// Collect only a provider-verified, stable snapshot. A missing continuation
/// token does NOT prove completion. Do not interpret any error here as a
/// legitimate empty directory or as permission to delete local files.
pub fn collect_verified_listing(
    remote: &dyn RemoteStore,
    page_limit: usize,
    max_pages: usize,
) -> Result<RemoteSnapshot, RemoteStoreError> {
    use std::collections::BTreeSet;

    if page_limit == 0 || max_pages == 0 {
        return Err(RemoteStoreError::InvalidCursor);
    }
    if !remote.capabilities().snapshot_consistency {
        return Err(RemoteStoreError::UnverifiableListing);
    }

    let mut snapshot_id: Option<String> = None;
    let mut cursor: Option<String> = None;
    let mut seen_cursors = BTreeSet::new();
    let mut seen_objects = BTreeSet::new();
    let mut objects = Vec::new();
    for _ in 0..max_pages {
        let page = remote.list_page(cursor.as_deref(), page_limit)?;
        if page.snapshot_id.is_empty() || page.objects.len() > page_limit {
            return Err(RemoteStoreError::InvalidListing);
        }
        if let Some(expected) = &snapshot_id {
            if expected != &page.snapshot_id {
                return Err(RemoteStoreError::ListingChanged);
            }
        } else {
            snapshot_id = Some(page.snapshot_id.clone());
        }
        for object in page.objects {
            if object.id.is_empty() || object.revision.is_empty() {
                return Err(RemoteStoreError::InvalidListing);
            }
            if !seen_objects.insert(object.id.clone()) {
                return Err(RemoteStoreError::DuplicateRemoteObject);
            }
            objects.push(object);
        }
        match (page.complete, page.next_cursor) {
            (true, None) => return Ok(RemoteSnapshot {
                snapshot_id: snapshot_id.unwrap_or_default(),
                objects,
            }),
            (true, Some(_)) => return Err(RemoteStoreError::InvalidListing),
            (false, None) => return Err(RemoteStoreError::IncompleteListing),
            (false, Some(next)) => {
                if next.is_empty() || !seen_cursors.insert(next.clone()) {
                    return Err(RemoteStoreError::InvalidCursor);
                }
                cursor = Some(next);
            }
        }
    }
    Err(RemoteStoreError::PageLimitExceeded)
}

pub trait RemoteStore {
    fn capabilities(&self) -> ProviderCapabilities;
    fn list_page(
        &self,
        continuation: Option<&str>,
        page_limit: usize,
    ) -> Result<RemotePage, RemoteStoreError>;
    fn read(&self, object_id: &str) -> Result<Vec<u8>, RemoteStoreError>;
    fn head(&self, object_id: &str) -> Result<RemoteObject, RemoteStoreError>;
    fn put(
        &mut self,
        object_id: &str,
        bytes: &[u8],
        precondition: &RemotePrecondition,
    ) -> Result<RemoteWriteReceipt, RemoteStoreError>;
    fn delete(
        &mut self,
        object_id: &str,
        precondition: &RemotePrecondition,
    ) -> Result<(), RemoteStoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_page_requires_verified_completeness() {
        let page = RemotePage {
            objects: vec![],
            next_cursor: None,
            complete: false,
            snapshot_id: "snapshot-1".into(),
        };
        assert!(!page.is_final_verified_page());
        assert!(RemotePage { complete: true, ..page }.is_final_verified_page());
    }

    #[test]
    fn weak_provider_is_not_guarded() {
        let weak = ProviderCapabilities {
            conditional_create: true,
            conditional_replace: false,
            conditional_delete: false,
            snapshot_consistency: false,
            version_history: false,
            delta_feed: false,
        };
        assert!(!weak.supports_guarded_mutation());
    }
}
