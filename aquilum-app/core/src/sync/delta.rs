//! Read-only, provider-neutral HAVE/WANT planning for immutable sync revisions.
//!
//! This is a metadata-only experiment. The caller must independently verify
//! that each announced revision inventory is complete and bound to a stable
//! local scan or immutable remote manifest. The snapshot label and complete
//! flag are declarations, NOT cryptographic proofs.
//!
//! The resulting object hashes are hints for later availability/readback checks,
//! not proof that a sender has bytes or permission to publish a remote head.
//! This module performs no network I/O, local mutation, CAS, or baseline commit.

use std::collections::{BTreeMap, BTreeSet};

use super::dag::{RevisionDag, RevisionOperation, RevisionRecord};
use super::{RevisionId, VaultId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionInventory {
    pub vault_id: VaultId,
    /// Identifies the verified local scan or immutable manifest generation.
    /// The caller must verify this anchor before constructing a trusted plan.
    pub snapshot_id: String,
    /// False if paging, scan, ancestry, or snapshot verification is unfinished.
    pub complete: bool,
    /// All advertised revisions, including their transitive parent closure.
    pub revisions: BTreeSet<RevisionId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeltaPlanError {
    VaultMismatch,
    EmptySnapshotIdentity,
    IncompleteInventory,
    UnknownRevision(RevisionId),
    MissingParent {
        revision: RevisionId,
        parent: RevisionId,
    },
    InvalidAncestry,
}

/// Immutable revision records the destination does not announce.
/// Revision records are parent-first; content hashes are sorted and deduplicated.
///
/// A content hash is a candidate object, not an instruction to PUT or GET.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MissingRevisionBatch {
    pub revisions: Vec<RevisionId>,
    pub candidate_blob_hashes: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RevisionExchangePlan {
    /// Candidate immutable metadata to send from local to remote.
    pub to_remote: MissingRevisionBatch,
    /// Candidate immutable metadata to receive from remote to local.
    pub to_local: MissingRevisionBatch,
}

fn content_hash(record: &RevisionRecord) -> Option<&str> {
    match &record.operation {
        RevisionOperation::Content { blob_hash, .. }
        | RevisionOperation::Rename { blob_hash, .. } => Some(blob_hash),
        RevisionOperation::Tombstone { .. } => None,
    }
}

fn validate_inventory(
    dag: &RevisionDag,
    inventory: &RevisionInventory,
) -> Result<(), DeltaPlanError> {
    if inventory.snapshot_id.is_empty()
        || inventory.snapshot_id.chars().any(char::is_control)
    {
        return Err(DeltaPlanError::EmptySnapshotIdentity);
    }
    if !inventory.complete {
        return Err(DeltaPlanError::IncompleteInventory);
    }
    for revision in &inventory.revisions {
        let record = dag
            .get(*revision)
            .ok_or(DeltaPlanError::UnknownRevision(*revision))?;
        for parent in &record.parents {
            if !inventory.revisions.contains(parent) {
                return Err(DeltaPlanError::MissingParent {
                    revision: *revision,
                    parent: *parent,
                });
            }
        }
    }
    Ok(())
}

/// Compare two *already verified* immutable revision inventories.
///
/// This is not a file-change planner: offline sibling revisions are retained
/// independently and must still pass plan_snapshot and File Gate checks later.
/// Missing revisions never imply a deletion; an absent payload must be checked
/// separately using the provider-specific verified object lookup.
pub fn plan_revision_exchange(
    dag: &RevisionDag,
    local: &RevisionInventory,
    remote: &RevisionInventory,
) -> Result<RevisionExchangePlan, DeltaPlanError> {
    if local.vault_id != remote.vault_id {
        return Err(DeltaPlanError::VaultMismatch);
    }
    validate_inventory(dag, local)?;
    validate_inventory(dag, remote)?;

    Ok(RevisionExchangePlan {
        to_remote: missing_batch(dag, &local.revisions, &remote.revisions)?,
        to_local: missing_batch(dag, &remote.revisions, &local.revisions)?,
    })
}

fn missing_batch(
    dag: &RevisionDag,
    source: &BTreeSet<RevisionId>,
    destination: &BTreeSet<RevisionId>,
) -> Result<MissingRevisionBatch, DeltaPlanError> {
    let wanted: BTreeSet<RevisionId> = source.difference(destination).copied().collect();
    let mut indegree: BTreeMap<RevisionId, usize> = BTreeMap::new();
    let mut children: BTreeMap<RevisionId, BTreeSet<RevisionId>> = BTreeMap::new();

    for revision in &wanted {
        let record = dag
            .get(*revision)
            .ok_or(DeltaPlanError::UnknownRevision(*revision))?;
        let mut dependencies = 0;
        for parent in &record.parents {
            if wanted.contains(parent) {
                dependencies += 1;
                children.entry(*parent).or_default().insert(*revision);
            }
        }
        indegree.insert(*revision, dependencies);
    }

    // Topological order with a deterministic ID tie-breaker, without recursion.
    let mut ready: BTreeSet<RevisionId> = indegree
        .iter()
        .filter_map(|(revision, count)| (*count == 0).then_some(*revision))
        .collect();
    let mut ordered = Vec::with_capacity(wanted.len());
    let mut hashes = BTreeSet::new();

    while let Some(revision) = ready.iter().next().copied() {
        ready.remove(&revision);
        ordered.push(revision);
        let record = dag
            .get(revision)
            .ok_or(DeltaPlanError::UnknownRevision(revision))?;
        if let Some(hash) = content_hash(record) {
            hashes.insert(hash.to_owned());
        }
        if let Some(descendants) = children.get(&revision) {
            for child in descendants {
                let remaining = indegree
                    .get_mut(child)
                    .ok_or(DeltaPlanError::InvalidAncestry)?;
                *remaining = remaining
                    .checked_sub(1)
                    .ok_or(DeltaPlanError::InvalidAncestry)?;
                if *remaining == 0 {
                    ready.insert(*child);
                }
            }
        }
    }
    if ordered.len() != wanted.len() {
        return Err(DeltaPlanError::InvalidAncestry);
    }

    Ok(MissingRevisionBatch {
        revisions: ordered,
        candidate_blob_hashes: hashes.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{DocumentId, RevisionRecord, SyncRelativePath};

    fn add_content(
        dag: &mut RevisionDag,
        document_id: DocumentId,
        parents: Vec<RevisionId>,
        bytes: &[u8],
    ) -> RevisionId {
        let revision = RevisionId::new();
        dag.ingest(RevisionRecord {
            id: revision,
            document_id,
            parents,
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse("notes/example.md").unwrap(),
                blob_hash: blake3::hash(bytes).to_hex().to_string(),
            },
        })
        .unwrap();
        revision
    }

    fn inventory(vault_id: VaultId, revisions: &[RevisionId]) -> RevisionInventory {
        RevisionInventory {
            vault_id,
            snapshot_id: "verified-fixture-generation".into(),
            complete: true,
            revisions: revisions.iter().copied().collect(),
        }
    }

    #[test]
    fn no_change_requires_no_revision_or_blob_exchange() {
        let mut dag = RevisionDag::new();
        let vault = VaultId::new();
        let root = add_content(&mut dag, DocumentId::new(), vec![], b"unchanged");
        let snapshot = inventory(vault, &[root]);
        let plan = plan_revision_exchange(&dag, &snapshot, &snapshot).unwrap();
        assert_eq!(plan, RevisionExchangePlan::default());
    }

    #[test]
    fn missing_revisions_are_parent_first_even_when_ids_sort_differently() {
        let mut dag = RevisionDag::new();
        let vault = VaultId::new();
        let document = DocumentId::new();
        let root = add_content(&mut dag, document, vec![], b"root");
        let second = add_content(&mut dag, document, vec![root], b"second");
        let third = add_content(&mut dag, document, vec![second], b"third");

        let result = plan_revision_exchange(
            &dag,
            &inventory(vault, &[root, second, third]),
            &inventory(vault, &[]),
        )
        .unwrap();
        assert_eq!(result.to_remote.revisions, vec![root, second, third]);
        assert!(result.to_local.revisions.is_empty());
        assert_eq!(result.to_remote.candidate_blob_hashes.len(), 3);
    }

    #[test]
    fn offline_siblings_are_exchanged_without_choosing_a_winner() {
        let mut dag = RevisionDag::new();
        let vault = VaultId::new();
        let doc = DocumentId::new();
        let common = add_content(&mut dag, doc, vec![], b"base");
        let left = add_content(&mut dag, doc, vec![common], b"left");
        let right = add_content(&mut dag, doc, vec![common], b"right");
        let plan = plan_revision_exchange(
            &dag,
            &inventory(vault, &[common, left]),
            &inventory(vault, &[common, right]),
        )
        .unwrap();
        assert_eq!(plan.to_remote.revisions, vec![left]);
        assert_eq!(plan.to_local.revisions, vec![right]);
    }

    #[test]
    fn tombstone_is_metadata_only_and_never_a_delete_instruction() {
        let mut dag = RevisionDag::new();
        let vault = VaultId::new();
        let doc = DocumentId::new();
        let base = add_content(&mut dag, doc, vec![], b"base");
        let deletion = RevisionId::new();
        dag.ingest(RevisionRecord {
            id: deletion,
            document_id: doc,
            parents: vec![base],
            operation: RevisionOperation::Tombstone {
                path: SyncRelativePath::parse("notes/example.md").unwrap(),
            },
        })
        .unwrap();
        let plan = plan_revision_exchange(
            &dag,
            &inventory(vault, &[base, deletion]),
            &inventory(vault, &[base]),
        )
        .unwrap();
        assert_eq!(plan.to_remote.revisions, vec![deletion]);
        assert!(plan.to_remote.candidate_blob_hashes.is_empty());
    }

    #[test]
    fn identical_content_in_multiple_revisions_is_one_blob_hint() {
        let mut dag = RevisionDag::new();
        let vault = VaultId::new();
        let a = add_content(&mut dag, DocumentId::new(), vec![], b"same");
        let b = add_content(&mut dag, DocumentId::new(), vec![], b"same");
        let plan = plan_revision_exchange(
            &dag,
            &inventory(vault, &[a, b]),
            &inventory(vault, &[]),
        )
        .unwrap();
        assert_eq!(plan.to_remote.revisions.len(), 2);
        assert_eq!(plan.to_remote.candidate_blob_hashes.len(), 1);
    }

    #[test]
    fn incomplete_foreign_or_unsupported_inventory_fails_closed() {
        let dag = RevisionDag::new();
        let vault = VaultId::new();
        let good = inventory(vault, &[]);
        let mut incomplete = good.clone();
        incomplete.complete = false;
        assert_eq!(
            plan_revision_exchange(&dag, &good, &incomplete),
            Err(DeltaPlanError::IncompleteInventory)
        );
        let mut nameless = good.clone();
        nameless.snapshot_id.clear();
        assert_eq!(
            plan_revision_exchange(&dag, &good, &nameless),
            Err(DeltaPlanError::EmptySnapshotIdentity)
        );
        assert_eq!(
            plan_revision_exchange(&dag, &good, &inventory(VaultId::new(), &[])),
            Err(DeltaPlanError::VaultMismatch)
        );
    }

    #[test]
    fn unknown_pending_and_missing_parent_revisions_are_rejected() {
        let mut dag = RevisionDag::new();
        let vault = VaultId::new();
        let unknown = RevisionId::new();
        assert_eq!(
            plan_revision_exchange(
                &dag,
                &inventory(vault, &[unknown]),
                &inventory(vault, &[]),
            ),
            Err(DeltaPlanError::UnknownRevision(unknown))
        );
        let document = DocumentId::new();
        let parent = add_content(&mut dag, document, vec![], b"parent");
        let child = add_content(&mut dag, document, vec![parent], b"child");
        assert_eq!(
            plan_revision_exchange(
                &dag,
                &inventory(vault, &[child]),
                &inventory(vault, &[]),
            ),
            Err(DeltaPlanError::MissingParent {
                revision: child,
                parent,
            })
        );
    }

    #[test]
    fn ten_thousand_stable_revisions_need_only_seven_new_revision_records() {
        let mut dag = RevisionDag::new();
        let vault = VaultId::new();
        let mut shared = Vec::new();
        let mut local = Vec::new();
        let mut changed = BTreeSet::new();
        for index in 0..10_000 {
            let document = DocumentId::new();
            let original = add_content(&mut dag, document, vec![], b"unchanged");
            shared.push(original);
            local.push(original);
            if index % 1_500 == 0 {
                let updated = add_content(&mut dag, document, vec![original], b"changed");
                local.push(updated);
                changed.insert(updated);
            }
        }
        let plan = plan_revision_exchange(
            &dag,
            &inventory(vault, &local),
            &inventory(vault, &shared),
        )
        .unwrap();
        assert_eq!(
            plan.to_remote.revisions.iter().copied().collect::<BTreeSet<_>>(),
            changed
        );
        assert_eq!(plan.to_remote.revisions.len(), 7);
        assert_eq!(plan.to_remote.candidate_blob_hashes.len(), 1);
        assert!(plan.to_local.revisions.is_empty());
    }
}
