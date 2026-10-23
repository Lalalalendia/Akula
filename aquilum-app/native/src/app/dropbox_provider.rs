//! Dropbox transport-to-Sync-Core boundary for read-only review.
//! Provider metadata is NOT an authenticated revision DAG or a BLAKE3
//! checksum. Never upgrade it into a verified snapshot or a File Gate grant.

use aquilum_core::sync::{
    plan_snapshot, DocumentId, HoldReason, ObservedObject, PlannedAction,
    ProviderCapabilities, RemoteObject, RemotePage, RemotePrecondition, RemoteStore,
    RemoteStoreError, RemoteWriteReceipt, RevisionDag, RevisionId, SafetyPolicy, SnapshotDiff,
};
use akula_dropbox_t0::{
    read_http::{AccessToken, DropboxReadOnly, ReadError},
    DropboxDeltaEntry, DropboxDeltaPage, Error as DropboxError, ObjectKey, Scope,
};

/// Native adapter. No cloud mutation, local file apply or baseline advancement.
pub(super) struct DropboxReadOnlyAdapter {
    client: DropboxReadOnly,
}

impl DropboxReadOnlyAdapter {
    pub(super) fn connect(scope: Scope, token: AccessToken) -> Result<Self, ReadError> {
        Ok(Self { client: DropboxReadOnly::connect(scope, token)? })
    }
}

fn capabilities() -> ProviderCapabilities {
    ProviderCapabilities {
        conditional_create: false,
        conditional_replace: false,
        conditional_delete: false,
        // Dropbox listing cursors alone do not establish the stable
        // signed snapshot required by collect_verified_listing.
        snapshot_consistency: false,
        version_history: false,
        // Tombstones cannot be promoted into Akula's signed DAG.
        delta_feed: false,
    }
}

fn project_page(
    page: DropboxDeltaPage,
    page_limit: usize,
) -> Result<RemotePage, RemoteStoreError> {
    if page_limit == 0 || page.entries.len() > page_limit {
        return Err(RemoteStoreError::InvalidListing);
    }
    let mut objects = Vec::with_capacity(page.entries.len());
    for item in page.entries {
        match item {
            DropboxDeltaEntry::File(file) => {
                if !file.downloadable { return Err(RemoteStoreError::PermissionDenied); }
                objects.push(RemoteObject {
                    // Only the scoped Akula logical key, never a Dropbox-native ID.
                    id: file.logical_key.as_str().to_owned(),
                    revision: file.revision.as_str().to_owned(),
                    byte_length: file.byte_length,
                    // Dropbox's block SHA-256 and native rev are NOT BLAKE3.
                    content_hash: None,
                });
            }
            // A Dropbox deleted entry is NOT an Akula signed tombstone.
            DropboxDeltaEntry::Deleted(_) => {
                return Err(RemoteStoreError::UnverifiableListing);
            }
        }
    }
    Ok(RemotePage {
        objects,
        next_cursor: page.has_more.then_some(page.cursor),
        complete: !page.has_more,
        snapshot_id: "DROPBOX_UNVERIFIED_TRANSPORT_ONLY".to_owned(),
    })
}

/// Dropbox metadata is only a transport hint. Until a remote revision is
/// admitted into Akula's authenticated DAG, the core planner must HOLD even
/// if a Dropbox cursor says that a file exists or was deleted.
pub(super) fn plan_transport_preview(
    document_id: DocumentId,
    local: ObservedObject,
    baseline: Option<RevisionId>,
    _remote: &RemoteObject,
    dag: &RevisionDag,
) -> PlannedAction {
    plan_snapshot(&SnapshotDiff {
        document_id,
        confirmed_baseline: baseline,
        local,
        remote: ObservedObject::Unknown,
    }, dag, SafetyPolicy::default())
}

fn map_error(error: ReadError) -> RemoteStoreError {
    match error {
        ReadError::AccountMismatch | ReadError::InvalidAccountMetadata
            | ReadError::Protocol(DropboxError::AuthRequired | DropboxError::PermissionDenied)
            | ReadError::NoDownloadPermission => RemoteStoreError::PermissionDenied,
        ReadError::Protocol(DropboxError::InvalidCursor) => RemoteStoreError::InvalidCursor,
        ReadError::Metadata(_) | ReadError::ResponseTooLarge => RemoteStoreError::InvalidListing,
        _ => RemoteStoreError::Unavailable,
    }
}

impl RemoteStore for DropboxReadOnlyAdapter {
    fn capabilities(&self) -> ProviderCapabilities { capabilities() }

    fn list_page(
        &self,
        cursor: Option<&str>,
        page_limit: usize,
    ) -> Result<RemotePage, RemoteStoreError> {
        if page_limit == 0 { return Err(RemoteStoreError::InvalidCursor); }
        let page = self.client.list_delta(cursor).map_err(map_error)?;
        project_page(page, page_limit)
    }

    fn read(&self, _object_id: &str) -> Result<Vec<u8>, RemoteStoreError> {
        // Cannot read without a trusted BLAKE3 pin and exact object revision.
        Err(RemoteStoreError::PreconditionUnsupported)
    }

    fn head(&self, _object_id: &str) -> Result<RemoteObject, RemoteStoreError> {
        // Native Dropbox file identity is not enough to verify an Akula HEAD.
        Err(RemoteStoreError::PreconditionUnsupported)
    }

    fn put(
        &mut self,
        _object_id: &str,
        _bytes: &[u8],
        _precondition: &RemotePrecondition,
    ) -> Result<RemoteWriteReceipt, RemoteStoreError> {
        Err(RemoteStoreError::PreconditionUnsupported)
    }

    fn delete(
        &mut self,
        _object_id: &str,
        _precondition: &RemotePrecondition,
    ) -> Result<(), RemoteStoreError> {
        Err(RemoteStoreError::PreconditionUnsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> Scope {
        Scope::new("dbid:alice", "12345",
            "12345678-1234-1234-1234-123456789abc", "default").unwrap()
    }
    fn page(entries: serde_json::Value, has_more: bool) -> DropboxDeltaPage {
        let wire = serde_json::json!({
            "entries": entries, "has_more": has_more, "cursor": "cursor-next"
        });
        akula_dropbox_t0::parse_delta_page(&scope(), &serde_json::to_vec(&wire).unwrap()).unwrap()
    }
    fn file() -> serde_json::Value {
        serde_json::json!({
            ".tag": "file",
            "path_lower": scope().object_path(&ObjectKey::head()),
            "id": "id:abc", "rev": "r1", "size": 42,
            "is_downloadable": true,
            "content_hash": "a".repeat(64)
        })
    }

    #[test]
    fn provider_has_no_mutation_or_trusted_snapshot_authority() {
        let caps = capabilities();
        assert!(!caps.conditional_create && !caps.conditional_replace);
        assert!(!caps.conditional_delete && !caps.snapshot_consistency);
        assert!(!caps.delta_feed && !caps.supports_guarded_mutation());
    }

    #[test]
    fn scoped_file_projection_never_promotes_dropbox_sha_to_blake3() {
        let first = project_page(page(serde_json::json!([file()]), true), 100).unwrap();
        assert_eq!(first.objects.len(), 1);
        assert_eq!(first.objects[0].id, "refs/current.json");
        assert_eq!(first.objects[0].revision, "r1");
        assert_eq!(first.objects[0].content_hash, None);
        assert!(!first.complete);
        assert_eq!(first.next_cursor.as_deref(), Some("cursor-next"));
    }

    #[test]
    fn unverified_dropbox_revision_cannot_become_a_download_or_delete_plan() {
        let remote = project_page(page(serde_json::json!([file()]), false), 100)
            .unwrap().objects.remove(0);
        let doc = DocumentId::new();
        let dag = RevisionDag::new();
        let action = plan_transport_preview(doc, ObservedObject::VerifiedAbsent,
            None, &remote, &dag);
        assert_eq!(action, PlannedAction::Hold(HoldReason::UntrustedObservation));
        assert_eq!(remote.revision, "r1");
    }

    #[test]
    fn deletions_missing_permissions_and_short_pages_cannot_be_applied() {
        let deleted = serde_json::json!([{
            ".tag": "deleted", "path_lower": scope().object_path(&ObjectKey::head())
        }]);
        assert_eq!(project_page(page(deleted, false), 100),
            Err(RemoteStoreError::UnverifiableListing));
        let too_small = project_page(page(serde_json::json!([file()]), false), 0);
        assert_eq!(too_small, Err(RemoteStoreError::InvalidListing));
        let too_many = project_page(page(serde_json::json!([file()]), false), 0);
        assert_eq!(too_many, Err(RemoteStoreError::InvalidListing));
    }
}
