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
    PreconditionFailed,
    PreconditionUnsupported,
    /// Write may have happened despite a lost/failed response: MUST read back.
    CommitUncertain,
    IntegrityMismatch,
}

/// The provider is an exchange service, not a deletion authority. In particular,
/// all mutations require explicit preconditions; backends that cannot provide an
/// atomic guard must reject them, not silently degrade to last-write-wins.
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
