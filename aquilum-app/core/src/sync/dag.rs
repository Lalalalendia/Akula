//! Stage 02: minimal immutable revision DAG for individual logical documents.
//!
//! This is NOT the mutable local HistoryService and never deletes files. Objects
//! can arrive out of order; children remain pending until their parents exist.
//! No remote/cloud IO occurs in this module.

use super::{DocumentId, RevisionId, SyncRelativePath};
use std::collections::{BTreeMap, BTreeSet};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RevisionOperation {
    /// A new immutable content blob, addressed by its BLAKE3 lowercase hex.
    Content { path: SyncRelativePath, blob_hash: String },
    /// A path change retaining the content identity. Not a destructive FS move.
    Rename {
        from: SyncRelativePath,
        to: SyncRelativePath,
        blob_hash: String,
    },
    /// A logical deletion proposal, not permission to remove any local bytes.
    Tombstone { path: SyncRelativePath },
}

impl RevisionOperation {
    fn validate(&self) -> Result<(), DagError> {
        let hash = match self {
            Self::Content { blob_hash, .. } | Self::Rename { blob_hash, .. } => blob_hash,
            Self::Tombstone { .. } => return Ok(()),
        };
        if hash.len() != 64
            || !hash.bytes().all(|ch| ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch))
        {
            return Err(DagError::InvalidBlobHash);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionRecord {
    pub id: RevisionId,
    pub document_id: DocumentId,
    pub parents: Vec<RevisionId>,
    pub operation: RevisionOperation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DagError {
    InvalidBlobHash,
    DuplicateParent,
    SelfParent,
    Cycle,
    RevisionCollision,
    CrossDocumentParent,
    UnknownRevision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IngestOutcome {
    /// Accepted into the active DAG; may include previously pending descendants.
    Activated(Vec<RevisionId>),
    /// Stored but NOT usable as an active revision/head until parents are known.
    Pending,
    /// Identical record already present, no change.
    AlreadyPresent,
}

#[derive(Debug, Default)]
pub struct RevisionDag {
    active: BTreeMap<RevisionId, RevisionRecord>,
    pending: BTreeMap<RevisionId, RevisionRecord>,
}

impl RevisionDag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active_len(&self) -> usize {
        self.active.len()
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// A pending child is not yet a verified revision/head. Consumers which
    /// need a complete local snapshot can fail closed for this one document
    /// without blocking unrelated documents that have missing ancestors.
    pub fn has_pending_for_document(&self, document_id: DocumentId) -> bool {
        self.pending.values().any(|record| record.document_id == document_id)
    }

    /// Host-only diagnostic for linking an on-disk note to a Sync document.
    /// All *current* claims on this path (including pending changes) count:
    /// two documents using the same path cannot be mapped by path/hash alone.
    /// This is not a durable identity registry and does not grant read access.
    pub(crate) fn current_path_claimants(&self, path: &SyncRelativePath) -> BTreeSet<DocumentId> {
        let documents: BTreeSet<DocumentId> = self.active.values()
            .chain(self.pending.values())
            .map(|record| record.document_id)
            .collect();
        let mut claimants = BTreeSet::new();
        for document in documents {
            let active_claim = self.heads(document).iter().any(|head| {
                self.active.get(head).is_some_and(|record| match &record.operation {
                    RevisionOperation::Content { path: at, .. } => at == path,
                    RevisionOperation::Rename { to, .. } => to == path,
                    RevisionOperation::Tombstone { path: at } => at == path,
                })
            });
            let pending_claim = self.pending.values().any(|record| {
                record.document_id == document && match &record.operation {
                    RevisionOperation::Content { path: at, .. } => at == path,
                    RevisionOperation::Rename { to, .. } => to == path,
                    RevisionOperation::Tombstone { path: at } => at == path,
                }
            });
            if active_claim || pending_claim {
                claimants.insert(document);
            }
        }
        claimants
    }

    pub fn get(&self, id: RevisionId) -> Option<&RevisionRecord> {
        // Do not expose children whose ancestry is not yet verified.
        self.active.get(&id)
    }

    pub fn is_pending(&self, id: RevisionId) -> bool {
        self.pending.contains_key(&id)
    }

    /// Idempotent ingestion. Same revision ID with different bytes/parents
    /// fails closed, including when the first copy is waiting on dependencies.
    pub fn ingest(&mut self, record: RevisionRecord) -> Result<IngestOutcome, DagError> {
        record.operation.validate()?;
        if record.parents.iter().any(|parent| *parent == record.id) {
            return Err(DagError::SelfParent);
        }
        let parents: BTreeSet<_> = record.parents.iter().copied().collect();
        if parents.len() != record.parents.len() {
            return Err(DagError::DuplicateParent);
        }

        if let Some(existing) = self.active.get(&record.id)
            .or_else(|| self.pending.get(&record.id))
        {
            return if existing == &record {
                Ok(IngestOutcome::AlreadyPresent)
            } else {
                Err(DagError::RevisionCollision)
            };
        }

        // A parent arriving AFTER a child must still match the logical
        // document. Otherwise pending cross-document edges could remain
        // permanently unvalidated after a restart.
        if self.pending.values().any(|child| {
            child.parents.contains(&record.id) && child.document_id != record.document_id
        }) {
            return Err(DagError::CrossDocumentParent);
        }

        // Validate ancestry that is already known; unresolved records are
        // staged rather than treated as competing legitimate roots.
        for parent_id in &record.parents {
            if let Some(parent) = self.active.get(parent_id).or_else(|| self.pending.get(parent_id)) {
                if parent.document_id != record.document_id {
                    return Err(DagError::CrossDocumentParent);
                }
            }
            if self.pending_path_reaches(*parent_id, record.id) {
                return Err(DagError::Cycle);
            }
        }

        let ready = record.parents.iter().all(|parent| self.active.contains_key(parent));
        if !ready {
            self.pending.insert(record.id, record);
            return Ok(IngestOutcome::Pending);
        }
        let id = record.id;
        self.active.insert(id, record);
        let mut activated = vec![id];
        loop {
            let ready: Vec<_> = self.pending.iter()
                .filter(|(_, record)| record.parents.iter().all(|parent| {
                    self.active.get(parent).is_some_and(|node| {
                        node.document_id == record.document_id
                    })
                }))
                .map(|(id, _)| *id)
                .collect();
            if ready.is_empty() {
                break;
            }
            for id in ready {
                if let Some(record) = self.pending.remove(&id) {
                    self.active.insert(id, record);
                    activated.push(id);
                }
            }
        }
        Ok(IngestOutcome::Activated(activated))
    }

    fn pending_path_reaches(&self, start: RevisionId, target: RevisionId) -> bool {
        let mut visited = BTreeSet::new();
        let mut queue = vec![start];
        while let Some(id) = queue.pop() {
            if id == target {
                return true;
            }
            if !visited.insert(id) {
                continue;
            }
            if let Some(node) = self.pending.get(&id) {
                queue.extend(node.parents.iter().copied());
            }
        }
        false
    }

    /// Only fully verified active nodes participate. A pending child must not
    /// cause an earlier, usable head to disappear.
    pub fn heads(&self, document_id: DocumentId) -> BTreeSet<RevisionId> {
        let mut heads: BTreeSet<_> = self.active.values()
            .filter(|node| node.document_id == document_id)
            .map(|node| node.id)
            .collect();
        for node in self.active.values().filter(|node| node.document_id == document_id) {
            for parent in &node.parents {
                heads.remove(parent);
            }
        }
        heads
    }

    pub fn is_ancestor(
        &self,
        ancestor: RevisionId,
        descendant: RevisionId,
    ) -> Result<bool, DagError> {
        let ancestor_record = self.active.get(&ancestor).ok_or(DagError::UnknownRevision)?;
        let descendant_record = self.active.get(&descendant).ok_or(DagError::UnknownRevision)?;
        if ancestor_record.document_id != descendant_record.document_id {
            return Ok(false);
        }
        let mut seen = BTreeSet::new();
        let mut queue = vec![descendant];
        while let Some(id) = queue.pop() {
            if !seen.insert(id) {
                continue;
            }
            if id == ancestor {
                return Ok(true);
            }
            if let Some(node) = self.active.get(&id) {
                queue.extend(node.parents.iter().copied());
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(input: &[u8]) -> String {
        blake3::hash(input).to_hex().to_string()
    }

    fn record(
        id: RevisionId,
        document: DocumentId,
        parents: Vec<RevisionId>,
        text: &[u8],
    ) -> RevisionRecord {
        RevisionRecord {
            id,
            document_id: document,
            parents,
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse("notes/example.md").unwrap(),
                blob_hash: hash(text),
            },
        }
    }

    #[test]
    fn pending_is_scoped_by_document_for_read_only_consumers() {
        let mut dag = RevisionDag::new();
        let known = DocumentId::new();
        let delayed = DocumentId::new();
        let k = RevisionId::new();
        let missing = RevisionId::new();
        let child = RevisionId::new();
        dag.ingest(record(k, known, vec![], b"known")).unwrap();
        assert_eq!(
            dag.ingest(record(child, delayed, vec![missing], b"pending")),
            Ok(IngestOutcome::Pending),
        );
        assert!(!dag.has_pending_for_document(known));
        assert!(dag.has_pending_for_document(delayed));
        assert_eq!(dag.heads(known), BTreeSet::from([k]));
        assert!(dag.heads(delayed).is_empty());
        dag.ingest(record(missing, delayed, vec![], b"root")).unwrap();
        assert!(!dag.has_pending_for_document(delayed));
        assert_eq!(dag.heads(delayed), BTreeSet::from([child]));
    }

    #[test]
    fn current_path_claimants_fail_closed_on_aliases_and_pending() {
        let mut dag = RevisionDag::new();
        let one = DocumentId::new();
        let other = DocumentId::new();
        let path = SyncRelativePath::parse("notes/example.md").unwrap();
        let root = RevisionId::new();
        dag.ingest(record(root, one, vec![], b"a")).unwrap();
        assert_eq!(dag.current_path_claimants(&path), BTreeSet::from([one]));
        let also = RevisionId::new();
        dag.ingest(record(also, other, vec![], b"a")).unwrap();
        assert_eq!(dag.current_path_claimants(&path), BTreeSet::from([one, other]));

        let pending_doc = DocumentId::new();
        let child = RevisionId::new();
        dag.ingest(record(child, pending_doc, vec![RevisionId::new()], b"b")).unwrap();
        assert_eq!(dag.current_path_claimants(&path), BTreeSet::from([one, other, pending_doc]));
    }

    #[test]
    fn offline_siblings_become_two_heads_and_merge_keeps_both_parents() {
        let mut dag = RevisionDag::new();
        let document = DocumentId::new();
        let r0 = RevisionId::new();
        let r1 = RevisionId::new();
        let r2 = RevisionId::new();
        let merge = RevisionId::new();
        assert_eq!(
            dag.ingest(record(r0, document, vec![], b"base")),
            Ok(IngestOutcome::Activated(vec![r0]))
        );
        dag.ingest(record(r1, document, vec![r0], b"edit A")).unwrap();
        dag.ingest(record(r2, document, vec![r0], b"edit B")).unwrap();
        assert_eq!(dag.heads(document), BTreeSet::from([r1, r2]));
        assert_eq!(dag.is_ancestor(r1, r2), Ok(false));
        assert_eq!(dag.is_ancestor(r0, r1), Ok(true));
        dag.ingest(record(merge, document, vec![r1, r2], b"merged")).unwrap();
        assert_eq!(dag.heads(document), BTreeSet::from([merge]));
        assert_eq!(dag.active_len(), 4);
        assert_eq!(dag.get(merge).unwrap().parents, vec![r1, r2]);
        assert_eq!(dag.is_ancestor(r1, merge), Ok(true));
        assert_eq!(dag.is_ancestor(r2, merge), Ok(true));
    }

    #[test]
    fn out_of_order_children_do_not_advance_heads() {
        let mut dag = RevisionDag::new();
        let document = DocumentId::new();
        let r0 = RevisionId::new();
        let r1 = RevisionId::new();
        let r2 = RevisionId::new();
        assert_eq!(dag.ingest(record(r2, document, vec![r1], b"child")), Ok(IngestOutcome::Pending));
        assert!(dag.heads(document).is_empty());
        assert_eq!(dag.ingest(record(r1, document, vec![r0], b"parent")), Ok(IngestOutcome::Pending));
        assert!(dag.get(r1).is_none());
        assert_eq!(
            dag.ingest(record(r0, document, vec![], b"root")),
            Ok(IngestOutcome::Activated(vec![r0, r1, r2]))
        );
        assert_eq!(dag.pending_len(), 0);
        assert_eq!(dag.heads(document), BTreeSet::from([r2]));
    }

    #[test]
    fn tombstone_and_offline_edit_are_both_retained() {
        let mut dag = RevisionDag::new();
        let document = DocumentId::new();
        let base = RevisionId::new();
        let deleted = RevisionId::new();
        let edited = RevisionId::new();
        dag.ingest(record(base, document, vec![], b"base")).unwrap();
        dag.ingest(RevisionRecord {
            id: deleted,
            document_id: document,
            parents: vec![base],
            operation: RevisionOperation::Tombstone {
                path: SyncRelativePath::parse("notes/example.md").unwrap(),
            },
        }).unwrap();
        dag.ingest(record(edited, document, vec![base], b"offline edit")).unwrap();
        assert_eq!(dag.heads(document), BTreeSet::from([deleted, edited]));
        assert!(dag.get(base).is_some());
    }

    #[test]
    fn duplicate_identity_must_have_exactly_the_same_immutable_record() {
        let mut dag = RevisionDag::new();
        let id = RevisionId::new();
        let doc = DocumentId::new();
        let first = record(id, doc, vec![], b"first");
        assert!(matches!(dag.ingest(first.clone()), Ok(IngestOutcome::Activated(_))));
        assert_eq!(dag.ingest(first), Ok(IngestOutcome::AlreadyPresent));
        assert_eq!(dag.ingest(record(id, doc, vec![], b"tampered")), Err(DagError::RevisionCollision));
        assert_eq!(dag.active_len(), 1);
    }

    #[test]
    fn pending_cycles_and_known_cross_document_parents_are_rejected() {
        let mut dag = RevisionDag::new();
        let doc = DocumentId::new();
        let other = DocumentId::new();
        let a = RevisionId::new();
        let b = RevisionId::new();
        assert_eq!(dag.ingest(record(a, doc, vec![b], b"A")), Ok(IngestOutcome::Pending));
        assert_eq!(dag.ingest(record(b, doc, vec![a], b"B")), Err(DagError::Cycle));
        let root = RevisionId::new();
        dag.ingest(record(root, other, vec![], b"foreign")).unwrap();
        assert_eq!(dag.ingest(record(RevisionId::new(), doc, vec![root], b"wrong doc")),
            Err(DagError::CrossDocumentParent));
    }

    #[test]
    fn a_parent_arriving_late_cannot_link_a_pending_child_from_another_document() {
        let mut dag = RevisionDag::new();
        let doc_a = DocumentId::new();
        let doc_b = DocumentId::new();
        let parent = RevisionId::new();
        let child = RevisionId::new();
        assert_eq!(
            dag.ingest(record(child, doc_a, vec![parent], b"child")),
            Ok(IngestOutcome::Pending)
        );
        assert_eq!(
            dag.ingest(record(parent, doc_b, vec![], b"foreign parent")),
            Err(DagError::CrossDocumentParent)
        );
        assert_eq!(dag.active_len(), 0);
        assert_eq!(dag.pending_len(), 1);
        assert_eq!(dag.ingest(record(parent, doc_a, vec![], b"valid parent")),
            Ok(IngestOutcome::Activated(vec![parent, child])));
    }

    #[test]
    fn validates_content_hashes_and_rejects_duplicate_parents() {
        let mut dag = RevisionDag::new();
        let doc = DocumentId::new();
        let root = RevisionId::new();
        let invalid = RevisionRecord {
            id: root,
            document_id: doc,
            parents: vec![],
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse("test.md").unwrap(),
                blob_hash: "not-a-hash".into(),
            },
        };
        assert_eq!(dag.ingest(invalid), Err(DagError::InvalidBlobHash));
        dag.ingest(record(root, doc, vec![], b"real")).unwrap();
        assert_eq!(dag.ingest(record(RevisionId::new(), doc, vec![root, root], b"dup")),
            Err(DagError::DuplicateParent));
    }

    #[test]
    fn revision_wire_round_trip_and_unknown_authority_fields_are_rejected() {
        let doc = DocumentId::new();
        let original = record(RevisionId::new(), doc, vec![], b"content");
        let json = serde_json::to_value(&original).unwrap();
        let decoded: RevisionRecord = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(decoded, original);
        let mut malicious = json;
        malicious.as_object_mut().unwrap()
            .insert("remote_wipe_allowed".into(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<RevisionRecord>(malicious).is_err());
    }

    #[test]
    fn pending_nodes_are_not_exposed_for_ancestor_decisions() {
        let mut dag = RevisionDag::new();
        let doc = DocumentId::new();
        let a = RevisionId::new();
        let b = RevisionId::new();
        dag.ingest(record(b, doc, vec![a], b"pending")).unwrap();
        assert_eq!(dag.is_ancestor(a, b), Err(DagError::UnknownRevision));
        assert_eq!(dag.active_len(), 0);
        assert_eq!(dag.pending_len(), 1);
    }
}
