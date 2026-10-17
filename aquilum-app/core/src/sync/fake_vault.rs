//! Stage 04a: deterministic, in-memory VaultPort test double.
//!
//! Does NOT write real filesystem paths, inspect symlinks or provide crash
//! durability. The actual Core File Gate host adapter remains stage 04b.
//! All destructive actions fail closed unless an exact, host-recorded
//! protective copy exists and the dirty-editor/hash guards still match.

use super::{
    LocalFileEntry, LocalFileSnapshot, OperationId, PreservationReceipt, SyncRelativePath,
    VaultFileKind, VaultPort, VaultPortError,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
struct FakeFile {
    bytes: Vec<u8>,
    kind: VaultFileKind,
}

struct Backup {
    path: SyncRelativePath,
    content_hash: String,
    bytes: Vec<u8>,
}

#[derive(Default)]
pub struct FakeVaultPort {
    files: BTreeMap<String, FakeFile>,
    dirty_editor_buffers: BTreeSet<String>,
    backups: BTreeMap<OperationId, Backup>,
    trash: BTreeMap<OperationId, (SyncRelativePath, Vec<u8>)>,
    fail_preserve: bool,
    fail_write: bool,
}

impl FakeVaultPort {
    pub fn new() -> Self { Self::default() }

    pub fn set_fail_preserve(&mut self, fail: bool) { self.fail_preserve = fail; }
    pub fn set_fail_write(&mut self, fail: bool) { self.fail_write = fail; }

    pub fn set_dirty_editor_buffer(&mut self, path: &SyncRelativePath, dirty: bool) {
        if dirty {
            self.dirty_editor_buffers.insert(path.as_str().to_owned());
        } else {
            self.dirty_editor_buffers.remove(path.as_str());
        }
    }

    pub fn backup_bytes(&self, operation: OperationId) -> Option<&[u8]> {
        self.backups.get(&operation).map(|record| record.bytes.as_slice())
    }

    pub fn trashed_bytes(&self, operation: OperationId) -> Option<&[u8]> {
        self.trash.get(&operation).map(|(_, bytes)| bytes.as_slice())
    }

    pub fn file_count(&self) -> usize { self.files.len() }

    /// Controlled adversarial external edit, deliberately bypassing File Gate.
    /// Valid only in tests; demonstrates that an old receipt/hash no longer
    /// authorizes a destructive operation on newly changed bytes.
    pub fn simulate_external_edit(
        &mut self, path: &SyncRelativePath, bytes: &[u8],
    ) -> Result<(), VaultPortError> {
        let target = self.files.get_mut(path.as_str()).ok_or(VaultPortError::NotFound)?;
        target.bytes = bytes.to_vec();
        Ok(())
    }

    fn hash(bytes: &[u8]) -> String {
        blake3::hash(bytes).to_hex().to_string()
    }

    fn check_dirty(&self, path: &SyncRelativePath) -> Result<(), VaultPortError> {
        if self.dirty_editor_buffers.contains(path.as_str()) {
            Err(VaultPortError::DirtyEditorBuffer)
        } else {
            Ok(())
        }
    }

    fn check_no_collision(
        &self, destination: &SyncRelativePath, excluded: Option<&SyncRelativePath>,
    ) -> Result<(), VaultPortError> {
        if self.files.keys().any(|raw| {
            let Some(path) = SyncRelativePath::parse(raw).ok() else { return true };
            path.collision_key() == destination.collision_key()
                && excluded.is_none_or(|old| old != &path)
        }) {
            return Err(VaultPortError::DestinationExists);
        }
        Ok(())
    }

    fn current_hash(&self, path: &SyncRelativePath) -> Result<String, VaultPortError> {
        self.files.get(path.as_str()).map(|f| Self::hash(&f.bytes))
            .ok_or(VaultPortError::NotFound)
    }

    fn check_guard(
        &self, path: &SyncRelativePath, expected_hash: &str,
        receipt: &PreservationReceipt, operation_id: OperationId,
    ) -> Result<(), VaultPortError> {
        self.check_dirty(path)?;
        if self.current_hash(path)? != expected_hash {
            return Err(VaultPortError::FileChanged);
        }
        if !receipt.applies_to(path, expected_hash, operation_id) {
            return Err(VaultPortError::PreservationFailed);
        }
        let Some(backup) = self.backups.get(&operation_id) else {
            return Err(VaultPortError::PreservationFailed);
        };
        if backup.path != *path
            || backup.content_hash != expected_hash
            || Self::hash(&backup.bytes) != expected_hash
        {
            return Err(VaultPortError::PreservationFailed);
        }
        Ok(())
    }
}

impl VaultPort for FakeVaultPort {
    fn list_snapshot(&mut self) -> Result<Vec<LocalFileEntry>, VaultPortError> {
        self.files.iter().map(|(name, file)| {
            let path = SyncRelativePath::parse(name).map_err(|_| VaultPortError::UnsafePath)?;
            Ok(LocalFileEntry {
                path,
                content_hash: Self::hash(&file.bytes),
                byte_length: file.bytes.len() as u64,
                kind: file.kind,
            })
        }).collect()
    }

    fn read_snapshot(&mut self, path: &SyncRelativePath)
        -> Result<LocalFileSnapshot, VaultPortError>
    {
        let file = self.files.get(path.as_str()).ok_or(VaultPortError::NotFound)?;
        Ok(LocalFileSnapshot {
            bytes: file.bytes.clone(),
            content_hash: Self::hash(&file.bytes),
        })
    }

    fn has_unsaved_editor_changes(&self, path: &SyncRelativePath)
        -> Result<bool, VaultPortError>
    {
        Ok(self.dirty_editor_buffers.contains(path.as_str()))
    }

    fn preserve_before_mutation(
        &mut self, path: &SyncRelativePath,
        expected_hash: &str, operation_id: OperationId,
    ) -> Result<PreservationReceipt, VaultPortError> {
        self.check_dirty(path)?;
        if self.fail_preserve {
            return Err(VaultPortError::PreservationFailed);
        }
        let file = self.files.get(path.as_str()).ok_or(VaultPortError::NotFound)?;
        if Self::hash(&file.bytes) != expected_hash {
            return Err(VaultPortError::FileChanged);
        }
        if let Some(backup) = self.backups.get(&operation_id) {
            if backup.path != *path || backup.content_hash != expected_hash {
                return Err(VaultPortError::PreservationFailed);
            }
            return Ok(PreservationReceipt::confirmed(path.clone(), expected_hash.to_owned(), operation_id));
        }
        self.backups.insert(operation_id, Backup {
            path: path.clone(),
            content_hash: expected_hash.to_owned(),
            bytes: file.bytes.clone(),
        });
        Ok(PreservationReceipt::confirmed(path.clone(), expected_hash.to_owned(), operation_id))
    }

    fn create_if_absent(
        &mut self, path: &SyncRelativePath, bytes: &[u8], _operation_id: OperationId,
    ) -> Result<String, VaultPortError> {
        self.check_dirty(path)?;
        self.check_no_collision(path, None)?;
        if self.fail_write {
            return Err(VaultPortError::Io("simulated destination write error".into()));
        }
        self.files.insert(path.as_str().to_owned(), FakeFile {
            bytes: bytes.to_vec(),
            kind: if path.as_str().ends_with(".md") {
                VaultFileKind::Markdown
            } else {
                VaultFileKind::Attachment
            },
        });
        Ok(Self::hash(bytes))
    }

    fn replace_if_unchanged(
        &mut self, path: &SyncRelativePath, expected_hash: &str, bytes: &[u8],
        receipt: &PreservationReceipt, operation_id: OperationId,
    ) -> Result<String, VaultPortError> {
        self.check_guard(path, expected_hash, receipt, operation_id)?;
        if self.fail_write {
            return Err(VaultPortError::Io("simulated destination write error".into()));
        }
        let file = self.files.get_mut(path.as_str()).ok_or(VaultPortError::NotFound)?;
        file.bytes = bytes.to_vec();
        Ok(Self::hash(bytes))
    }

    fn rename_if_unchanged(
        &mut self, old_path: &SyncRelativePath, new_path: &SyncRelativePath,
        expected_hash: &str, receipt: &PreservationReceipt, operation_id: OperationId,
    ) -> Result<(), VaultPortError> {
        self.check_guard(old_path, expected_hash, receipt, operation_id)?;
        self.check_dirty(new_path)?;
        if old_path == new_path { return Ok(()) }
        self.check_no_collision(new_path, Some(old_path))?;
        if self.fail_write {
            return Err(VaultPortError::Io("simulated rename error".into()));
        }
        let file = self.files.remove(old_path.as_str()).ok_or(VaultPortError::NotFound)?;
        self.files.insert(new_path.as_str().to_owned(), file);
        Ok(())
    }

    fn trash_if_unchanged(
        &mut self, path: &SyncRelativePath, expected_hash: &str,
        receipt: &PreservationReceipt, operation_id: OperationId,
    ) -> Result<(), VaultPortError> {
        self.check_guard(path, expected_hash, receipt, operation_id)?;
        if self.fail_write {
            return Err(VaultPortError::Io("simulated trash error".into()));
        }
        if self.trash.contains_key(&operation_id) {
            return Err(VaultPortError::PreservationFailed);
        }
        let file = self.files.remove(path.as_str()).ok_or(VaultPortError::NotFound)?;
        self.trash.insert(operation_id, (path.clone(), file.bytes));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{
        plan_snapshot, DocumentId, FakeRemoteStore, ObservedObject, PlannedAction, RemotePrecondition,
        RemoteStore, RevisionDag, RevisionId, RevisionOperation, RevisionRecord, SafetyPolicy,
        SnapshotDiff,
    };

    fn path(s: &str) -> SyncRelativePath {
        SyncRelativePath::parse(s).unwrap()
    }

    fn init(vault: &mut FakeVaultPort, path: &SyncRelativePath, contents: &[u8]) -> String {
        vault.create_if_absent(path, contents, OperationId::new()).unwrap()
    }

    #[test]
    fn failed_preservation_blocks_overwrite_rename_and_trash() {
        let p = path("notes/a.md");
        let mut vault = FakeVaultPort::new();
        let original = init(&mut vault, &p, b"original bytes");
        vault.set_fail_preserve(true);
        let op = OperationId::new();
        assert_eq!(vault.preserve_before_mutation(&p, &original, op),
            Err(VaultPortError::PreservationFailed));
        // A receipt fabricated for a different operation cannot bypass a
        // missing durable protective copy.
        let other = PreservationReceipt::confirmed(p.clone(), original.clone(), OperationId::new());
        assert_eq!(vault.replace_if_unchanged(&p, &original, b"new", &other, op),
            Err(VaultPortError::PreservationFailed));
        assert_eq!(vault.rename_if_unchanged(&p, &path("notes/b.md"), &original, &other, op),
            Err(VaultPortError::PreservationFailed));
        assert_eq!(vault.trash_if_unchanged(&p, &original, &other, op),
            Err(VaultPortError::PreservationFailed));
        assert_eq!(vault.read_snapshot(&p).unwrap().bytes, b"original bytes");
        assert_eq!(vault.file_count(), 1);
    }

    #[test]
    fn a_dirty_editor_blocks_all_mutation_even_when_disk_hash_matches() {
        let p = path("notes/edit.md");
        let mut vault = FakeVaultPort::new();
        let hash = init(&mut vault, &p, b"on disk");
        vault.set_dirty_editor_buffer(&p, true);
        assert_eq!(vault.preserve_before_mutation(&p, &hash, OperationId::new()),
            Err(VaultPortError::DirtyEditorBuffer));
        assert_eq!(vault.has_unsaved_editor_changes(&p), Ok(true));
        vault.set_dirty_editor_buffer(&p, false);
        let op = OperationId::new();
        let receipt = vault.preserve_before_mutation(&p, &hash, op).unwrap();
        vault.set_dirty_editor_buffer(&p, true);
        assert_eq!(vault.replace_if_unchanged(&p, &hash, b"overwrite", &receipt, op),
            Err(VaultPortError::DirtyEditorBuffer));
        assert_eq!(vault.read_snapshot(&p).unwrap().bytes, b"on disk");
    }

    #[test]
    fn external_change_invalidates_old_hash_and_protection_receipt() {
        let p = path("notes/a.md");
        let mut vault = FakeVaultPort::new();
        let hash = init(&mut vault, &p, b"first");
        let op = OperationId::new();
        let receipt = vault.preserve_before_mutation(&p, &hash, op).unwrap();
        vault.simulate_external_edit(&p, b"changed externally").unwrap();
        assert_eq!(vault.replace_if_unchanged(&p, &hash, b"ours", &receipt, op),
            Err(VaultPortError::FileChanged));
        assert_eq!(vault.read_snapshot(&p).unwrap().bytes, b"changed externally");
        assert_eq!(vault.backup_bytes(op), Some(&b"first"[..]));
    }

    #[test]
    fn receipt_cannot_authorize_another_path_revision_or_operation() {
        let p = path("a.md");
        let q = path("b.md");
        let mut vault = FakeVaultPort::new();
        let hash = init(&mut vault, &p, b"first");
        init(&mut vault, &q, b"second");
        let op = OperationId::new();
        let receipt = vault.preserve_before_mutation(&p, &hash, op).unwrap();
        assert_eq!(vault.trash_if_unchanged(&p, &hash, &receipt, OperationId::new()),
            Err(VaultPortError::PreservationFailed));
        assert_eq!(vault.replace_if_unchanged(&q, &hash, b"wrong", &receipt, op),
            Err(VaultPortError::FileChanged));
        assert_eq!(vault.backup_bytes(op), Some(&b"first"[..]));
    }

    #[test]
    fn renamed_or_trashed_files_always_keep_protective_bytes() {
        let p = path("notes/old.md");
        let q = path("notes/new.md");
        let mut vault = FakeVaultPort::new();
        let hash = init(&mut vault, &p, b"historical");
        let op = OperationId::new();
        let receipt = vault.preserve_before_mutation(&p, &hash, op).unwrap();
        vault.rename_if_unchanged(&p, &q, &hash, &receipt, op).unwrap();
        assert_eq!(vault.backup_bytes(op), Some(&b"historical"[..]));
        assert!(matches!(vault.read_snapshot(&p), Err(VaultPortError::NotFound)));
        assert_eq!(vault.read_snapshot(&q).unwrap().bytes, b"historical");
        let op2 = OperationId::new();
        let receipt2 = vault.preserve_before_mutation(&q, &hash, op2).unwrap();
        vault.trash_if_unchanged(&q, &hash, &receipt2, op2).unwrap();
        assert_eq!(vault.trashed_bytes(op2), Some(&b"historical"[..]));
        assert_eq!(vault.file_count(), 0);
    }

    #[test]
    fn portable_name_collision_prevents_second_create_or_rename() {
        let mut vault = FakeVaultPort::new();
        let p = path("Notes/Caf\u{e9}.md");
        let q = path("notes/cafe\u{301}.md");
        let hash = init(&mut vault, &p, b"first");
        assert_eq!(vault.create_if_absent(&q, b"second", OperationId::new()),
            Err(VaultPortError::DestinationExists));
        let r = path("another.md");
        let rhash = init(&mut vault, &r, b"other");
        let op = OperationId::new();
        let ticket = vault.preserve_before_mutation(&r, &rhash, op).unwrap();
        assert_eq!(vault.rename_if_unchanged(&r, &q, &rhash, &ticket, op),
            Err(VaultPortError::DestinationExists));
        assert_eq!(vault.current_hash(&p).unwrap(), hash);
    }

    #[test]
    fn failed_write_keeps_data_and_confirmed_protective_copy() {
        let p = path("notes/file.md");
        let mut vault = FakeVaultPort::new();
        let hash = init(&mut vault, &p, b"old");
        let op = OperationId::new();
        let receipt = vault.preserve_before_mutation(&p, &hash, op).unwrap();
        vault.set_fail_write(true);
        assert!(matches!(vault.replace_if_unchanged(&p, &hash, b"new", &receipt, op),
            Err(VaultPortError::Io(_))));
        assert_eq!(vault.read_snapshot(&p).unwrap().bytes, b"old");
        assert_eq!(vault.backup_bytes(op), Some(&b"old"[..]));
    }

    #[test]
    fn two_clients_same_baseline_maintain_both_offline_heads() {
        let p = path("book/part.md");
        let doc = DocumentId::new();
        let mut a = FakeVaultPort::new();
        let mut b = FakeVaultPort::new();
        let mut remote = FakeRemoteStore::new();
        let mut dag = RevisionDag::new();
        let root = RevisionId::new();
        let left = RevisionId::new();
        let right = RevisionId::new();
        let base_hash = init(&mut a, &p, b"base");
        init(&mut b, &p, b"base");
        remote.put("revisions/root", b"base", &RemotePrecondition::Absent).unwrap();
        dag.ingest(RevisionRecord {
            id: root, document_id: doc, parents: vec![],
            operation: RevisionOperation::Content { path: p.clone(), blob_hash: base_hash.clone() },
        }).unwrap();

        // Client A and client B edit while disconnected. A deliberately cannot
        // modify B's local bytes and vice versa.
        let aop = OperationId::new();
        let aticket = a.preserve_before_mutation(&p, &base_hash, aop).unwrap();
        let a_hash = a.replace_if_unchanged(&p, &base_hash, b"A edit", &aticket, aop).unwrap();
        let bop = OperationId::new();
        let bticket = b.preserve_before_mutation(&p, &base_hash, bop).unwrap();
        let b_hash = b.replace_if_unchanged(&p, &base_hash, b"B edit", &bticket, bop).unwrap();
        for (id, hash) in [(left, a_hash.clone()), (right, b_hash.clone())] {
            dag.ingest(RevisionRecord {
                id, document_id: doc, parents: vec![root],
                operation: RevisionOperation::Content { path: p.clone(), blob_hash: hash },
            }).unwrap();
        }
        remote.put("revisions/left", b"A edit", &RemotePrecondition::Absent).unwrap();
        remote.put("revisions/right", b"B edit", &RemotePrecondition::Absent).unwrap();

        let plan = plan_snapshot(&SnapshotDiff {
            document_id: doc, confirmed_baseline: Some(root),
            local: ObservedObject::Present { revision: left, content_hash: a_hash },
            remote: ObservedObject::Present { revision: right, content_hash: b_hash },
        }, &dag, SafetyPolicy::default());
        assert_eq!(plan, PlannedAction::PreserveConflict { local: left, remote: right });
        assert_eq!(a.read_snapshot(&p).unwrap().bytes, b"A edit");
        assert_eq!(b.read_snapshot(&p).unwrap().bytes, b"B edit");
        assert_eq!(remote.object_count(), 3);
        assert_eq!(a.backup_bytes(aop), Some(&b"base"[..]));
        assert_eq!(b.backup_bytes(bop), Some(&b"base"[..]));
    }
}
