//! Host-facing vault contract. The sync engine must never write Markdown,
//! attachments or history directly, bypassing the Core File Gate.
//!
//! Implementation is deferred to PR 04: the current PR defines the explicit
//! preconditions that any implementation must satisfy.

use super::{OperationId, SyncRelativePath};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VaultFileKind {
    Markdown,
    Attachment,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalFileEntry {
    pub path: SyncRelativePath,
    /// Hash of on-disk bytes, not the editor's unsaved buffer.
    pub content_hash: String,
    pub byte_length: u64,
    pub kind: VaultFileKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalFileSnapshot {
    pub bytes: Vec<u8>,
    pub content_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VaultPortError {
    NotFound,
    DestinationExists,
    DirtyEditorBuffer,
    FileChanged,
    PreservationFailed,
    UnsafePath,
    PermissionDenied,
    Io(String),
}

/// A protective copy must exist durably BEFORE any destructive change.
/// A receipt is tied to one original path, one exact content hash and one
/// logical operation; another file/edit/delete must not reuse it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreservationReceipt {
    path: SyncRelativePath,
    content_hash: String,
    operation_id: OperationId,
}

impl PreservationReceipt {
    /// Called only by the trusted host after confirming durable preservation.
    /// The sync planner cannot manufacture a receipt.
    #[allow(dead_code)]
    pub(crate) fn confirmed(
        path: SyncRelativePath,
        content_hash: String,
        operation_id: OperationId,
    ) -> Self {
        Self { path, content_hash, operation_id }
    }

    pub fn applies_to(
        &self,
        path: &SyncRelativePath,
        content_hash: &str,
        operation_id: OperationId,
    ) -> bool {
        &self.path == path
            && self.content_hash == content_hash
            && self.operation_id == operation_id
    }
}

/// All mutating implementations MUST hold the host's file/write guard for
/// the final recheck and operation, defend against symlink TOCTOU, and reject
/// dirty editor buffers even if the on-disk hash is still unchanged.
/// "Preserve" must produce a durable copy or fail; logging a failed attempt
/// does NOT count as preservation.
pub trait VaultPort {
    fn list_snapshot(&mut self) -> Result<Vec<LocalFileEntry>, VaultPortError>;
    fn read_snapshot(
        &mut self,
        path: &SyncRelativePath,
    ) -> Result<LocalFileSnapshot, VaultPortError>;
    fn has_unsaved_editor_changes(
        &self,
        path: &SyncRelativePath,
    ) -> Result<bool, VaultPortError>;
    fn preserve_before_mutation(
        &mut self,
        path: &SyncRelativePath,
        expected_hash: &str,
        operation_id: OperationId,
    ) -> Result<PreservationReceipt, VaultPortError>;
    /// Only for creation: fails when an object already exists.
    fn create_if_absent(
        &mut self,
        path: &SyncRelativePath,
        bytes: &[u8],
        operation_id: OperationId,
    ) -> Result<String, VaultPortError>;
    /// Existing content only: requires durable preserve and a fresh hash check.
    fn replace_if_unchanged(
        &mut self,
        path: &SyncRelativePath,
        expected_hash: &str,
        bytes: &[u8],
        receipt: &PreservationReceipt,
        operation_id: OperationId,
    ) -> Result<String, VaultPortError>;
    fn rename_if_unchanged(
        &mut self,
        old_path: &SyncRelativePath,
        new_path: &SyncRelativePath,
        expected_hash: &str,
        receipt: &PreservationReceipt,
        operation_id: OperationId,
    ) -> Result<(), VaultPortError>;
    /// Soft-delete into recoverable trash, never direct unlink.
    fn trash_if_unchanged(
        &mut self,
        path: &SyncRelativePath,
        expected_hash: &str,
        receipt: &PreservationReceipt,
        operation_id: OperationId,
    ) -> Result<(), VaultPortError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protective_receipts_are_bound_to_one_file_hash_and_operation() {
        let path = SyncRelativePath::parse("notes/a.md").unwrap();
        let another = SyncRelativePath::parse("notes/b.md").unwrap();
        let operation = OperationId::new();
        let other_operation = OperationId::new();
        let receipt =
            PreservationReceipt::confirmed(path.clone(), "old-hash".into(), operation);
        assert!(receipt.applies_to(&path, "old-hash", operation));
        assert!(!receipt.applies_to(&another, "old-hash", operation));
        assert!(!receipt.applies_to(&path, "new-hash", operation));
        assert!(!receipt.applies_to(&path, "old-hash", other_operation));
    }
}
