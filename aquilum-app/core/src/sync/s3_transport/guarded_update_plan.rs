//! SYNC-APPLY-P0b: strictly read-only existing-file S3 fast-forward proposal.
//!
//! Reuse the existing pure plan_snapshot and fully verified, complete S3
//! metadata store. This is NOT permission to mutate a file, acknowledge
//! StageDownload, or advance any checkpoint. Head/ETag, protected original,
//! journal, editor and OS custody are independently rechecked at apply time.

use super::{S3HeadError, S3VerifiedRevisionMetadataStore};
use crate::sync::{
    plan_snapshot, ConfirmedBaseline, JournalAction, ObservedObject,
    OperationIntent, PlannedAction, RevisionDag, RevisionOperation,
    RevisionRecord, S3Keyspace, SafetyPolicy, SnapshotDiff, SyncRelativePath,
    RevisionId,
};

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum S3GuardedUpdatePlanError {
    UnverifiedSnapshot(S3HeadError),
    WrongVaultOrIntent,
    MalformedRecord,
    IncompleteHistory,
    AmbiguousDocumentHistory,
    Collision,
    NotFastForward,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct S3GuardedUpdatePreview {
    pub snapshot_id: String,
    pub previous_revision: RevisionId,
    pub next_revision: RevisionId,
    pub path: SyncRelativePath,
    pub previous_hash: String,
    pub next_hash: String,
}

/// Only a fully materialized, complete canonical V1 S3 metadata snapshot is
/// eligible; a V2 selective advisory preview, a raw S3 LIST, and a caller's
/// self-declared revision list cannot call this API. The previous per-document
/// baseline still must come from the protected local journal, not the network.
///
/// The DAG may contain many other documents, but NO historical revision of
/// another document may claim the same portable target path. For this one
/// document, every ancestor must have a Content operation at this exact path.
/// This intentionally HOLDS all rename/tombstone/sibling histories pending
/// a separately reviewed per-operation policy, even if a later Content tip
/// would otherwise appear harmless.
#[allow(dead_code)]
pub(crate) fn preview_verified_s3_existing_content_update(
    verified: &S3VerifiedRevisionMetadataStore,
    keyspace: &S3Keyspace,
    baseline: &ConfirmedBaseline,
    intent: &OperationIntent,
) -> Result<S3GuardedUpdatePreview, S3GuardedUpdatePlanError> {
    use S3GuardedUpdatePlanError as Error;
    if baseline.vault_id != keyspace.vault_id()
        || intent.vault_id != baseline.vault_id
        || intent.document_id != baseline.document_id
        || intent.profile_id != baseline.profile_id
        || intent.action != JournalAction::StageDownload
        || intent.remote_guard.is_some()
        || intent.expected_baseline != Some(baseline.revision_id)
        || intent.revision_id == baseline.revision_id
        || keyspace.immutable_blob_key(&intent.expected_content_hash)
            .ok().as_deref() != Some(intent.remote_object_id.as_str())
        || intent.path.as_str().split('/').any(|part| {
            part.eq_ignore_ascii_case(".aquilum")
                || part.eq_ignore_ascii_case(".trash")
                || part.eq_ignore_ascii_case(".git")
        })
    {
        return Err(Error::WrongVaultOrIntent);
    }
    let inventory = verified.verified_revision_inventory()
        .map_err(Error::UnverifiedSnapshot)?;
    if !inventory.complete || inventory.vault_id != baseline.vault_id
        || !inventory.revisions.contains(&baseline.revision_id)
        || !inventory.revisions.contains(&intent.revision_id)
    {
        return Err(Error::IncompleteHistory);
    }

    let mut dag = RevisionDag::new();
    let mut examined = 0usize;
    let destination = intent.path.collision_key();
    for (reference, json) in verified.verified_entries() {
        let record: RevisionRecord = serde_json::from_slice(json)
            .map_err(|_| Error::MalformedRecord)?;
        if reference.revision_id != record.id
            || reference.document_id != record.document_id
            || reference.revision_blake3 != blake3::hash(json).to_hex().to_string()
            || !inventory.revisions.contains(&record.id)
        {
            return Err(Error::MalformedRecord);
        }
        if record.document_id == intent.document_id {
            match &record.operation {
                RevisionOperation::Content { path, .. } if path == &intent.path => {}
                _ => return Err(Error::AmbiguousDocumentHistory),
            }
        } else {
            let collides = match &record.operation {
                RevisionOperation::Content { path, .. }
                | RevisionOperation::Tombstone { path } => {
                    path.collision_key() == destination
                }
                RevisionOperation::Rename { from, to, .. } => {
                    from.collision_key() == destination
                        || to.collision_key() == destination
                }
            };
            if collides {
                return Err(Error::Collision);
            }
        }
        dag.ingest(record).map_err(|_| Error::MalformedRecord)?;
        examined += 1;
    }
    if examined != inventory.revisions.len()
        || dag.pending_len() != 0 || dag.active_len() != examined
    {
        return Err(Error::IncompleteHistory);
    }
    let heads = dag.heads(intent.document_id);
    if heads.len() != 1 || !heads.contains(&intent.revision_id) {
        return Err(Error::AmbiguousDocumentHistory);
    }
    let expected = &intent.path;
    let previous = dag.get(baseline.revision_id)
        .ok_or(Error::IncompleteHistory)?;
    let next = dag.get(intent.revision_id)
        .ok_or(Error::IncompleteHistory)?;
    if !matches!(
        &previous.operation,
        RevisionOperation::Content { path, blob_hash }
            if path == expected && blob_hash == &baseline.content_hash
    ) || !matches!(
        &next.operation,
        RevisionOperation::Content { path, blob_hash }
            if path == expected && blob_hash == &intent.expected_content_hash
    ) {
        return Err(Error::NotFastForward);
    }

    // Existing provider-neutral policy remains the source of truth for
    // ancestry and when two content heads may be treated as fast-forward.
    // There is no new last-writer-wins, delete or merge rule here.
    let diff = SnapshotDiff {
        document_id: intent.document_id,
        confirmed_baseline: Some(baseline.revision_id),
        local: ObservedObject::Present {
            revision: baseline.revision_id, content_hash: baseline.content_hash.clone(),
        },
        remote: ObservedObject::Present {
            revision: intent.revision_id, content_hash: intent.expected_content_hash.clone(),
        },
    };
    if plan_snapshot(&diff, &dag, SafetyPolicy::default())
        != (PlannedAction::StageDownload { revision: intent.revision_id })
    {
        return Err(Error::NotFastForward);
    }
    Ok(S3GuardedUpdatePreview {
        snapshot_id: inventory.snapshot_id,
        previous_revision: baseline.revision_id,
        next_revision: intent.revision_id,
        path: intent.path.clone(),
        previous_hash: baseline.content_hash.clone(),
        next_hash: intent.expected_content_hash.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::S3VerifiedRevisionMetadataStore;
    use crate::sync::{
        DocumentId, OperationId, RevisionId, S3RevisionRefV1,
        SyncRelativePath, VaultId,
    };

    fn content(
        id: RevisionId, doc: DocumentId, parents: Vec<RevisionId>,
        path: &str, bytes: &[u8],
    ) -> RevisionRecord {
        RevisionRecord {
            id, document_id: doc, parents,
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse(path).unwrap(),
                blob_hash: blake3::hash(bytes).to_hex().to_string(),
            },
        }
    }

    fn snapshot(vault: VaultId, records: &[RevisionRecord]) -> S3VerifiedRevisionMetadataStore {
        let mut store = S3VerifiedRevisionMetadataStore::new(
            vault, "verified-root-etag".into(), 1_048_576,
        ).unwrap();
        for record in records {
            let json = serde_json::to_vec(record).unwrap();
            let blob_bytes = match record.operation {
                RevisionOperation::Tombstone { .. } => None,
                RevisionOperation::Content { .. } | RevisionOperation::Rename { .. } => Some(1),
            };
            store.accept(S3RevisionRefV1 {
                revision_id: record.id,
                document_id: record.document_id,
                revision_blake3: blake3::hash(&json).to_hex().to_string(),
                blob_bytes,
            }, json).unwrap();
        }
        store.validate_complete_dag().unwrap();
        store
    }

    fn accepted() -> (
        S3Keyspace, ConfirmedBaseline, OperationIntent,
        RevisionRecord, RevisionRecord,
    ) {
        let ns = S3Keyspace::new("akula/reviewed-content", VaultId::new()).unwrap();
        let doc = DocumentId::new();
        let old = content(RevisionId::new(), doc, vec![], "notes/one.md", b"old");
        let tip = content(RevisionId::new(), doc, vec![old.id], "notes/one.md", b"new");
        let old_hash = match &old.operation {
            RevisionOperation::Content { blob_hash, .. } => blob_hash.clone(),
            _ => unreachable!(),
        };
        let new_hash = match &tip.operation {
            RevisionOperation::Content { blob_hash, .. } => blob_hash.clone(),
            _ => unreachable!(),
        };
        let baseline = ConfirmedBaseline {
            vault_id: ns.vault_id(), document_id: doc,
            profile_id: "approved-s3".into(),
            revision_id: old.id, content_hash: old_hash, operation_id: OperationId::new(),
        };
        let intent = OperationIntent {
            operation_id: OperationId::new(),
            vault_id: ns.vault_id(), document_id: doc,
            profile_id: "approved-s3".into(), revision_id: tip.id,
            action: JournalAction::StageDownload,
            path: SyncRelativePath::parse("notes/one.md").unwrap(),
            remote_object_id: ns.immutable_blob_key(&new_hash).unwrap(),
            remote_guard: None, expected_content_hash: new_hash,
            expected_baseline: Some(old.id),
        };
        (ns, baseline, intent, old, tip)
    }

    #[test]
    fn accepted_single_descendant_has_no_write_or_baseline_effect() {
        let (ns, base, intent, old, tip) = accepted();
        let store = snapshot(ns.vault_id(), &[tip, old]);
        let p = preview_verified_s3_existing_content_update(
            &store, &ns, &base, &intent,
        ).unwrap();
        assert_eq!(p.previous_revision, base.revision_id);
        assert_eq!(p.next_revision, intent.revision_id);
        assert_eq!(p.previous_hash, base.content_hash);
        assert_eq!(p.next_hash, intent.expected_content_hash);
        assert_eq!(p.snapshot_id, store.snapshot_id());
    }

    #[test]
    fn wrong_vault_profile_baseline_and_blob_key_hold() {
        let (ns, base, intent, old, tip) = accepted();
        let store = snapshot(ns.vault_id(), &[old, tip]);
        let mut bad = intent.clone();
        bad.profile_id = "unapproved".into();
        assert!(preview_verified_s3_existing_content_update(
            &store, &ns, &base, &bad).is_err());
        let mut bad = intent.clone();
        bad.expected_baseline = Some(RevisionId::new());
        assert!(preview_verified_s3_existing_content_update(
            &store, &ns, &base, &bad).is_err());
        let mut bad = intent.clone();
        bad.remote_object_id = "foreign/object".into();
        assert!(preview_verified_s3_existing_content_update(
            &store, &ns, &base, &bad).is_err());
        let wrong_ns = S3Keyspace::new("other/vault", VaultId::new()).unwrap();
        assert!(preview_verified_s3_existing_content_update(
            &store, &wrong_ns, &base, &intent).is_err());
        let untrusted = S3VerifiedRevisionMetadataStore::new(
            ns.vault_id(), "unvalidated".into(), 1_048_576,
        ).unwrap();
        assert!(preview_verified_s3_existing_content_update(
            &untrusted, &ns, &base, &intent).is_err());
    }

    #[test]
    fn reject_siblings_tombstones_renames_and_foreign_casefolded_name() {
        let (ns, base, intent, old, tip) = accepted();
        let sibling = content(
            RevisionId::new(), old.document_id, vec![old.id],
            "notes/one.md", b"alternate",
        );
        let sibling_store = snapshot(ns.vault_id(), &[old.clone(), tip.clone(), sibling]);
        assert!(preview_verified_s3_existing_content_update(
            &sibling_store, &ns, &base, &intent).is_err());
        let foreign = content(
            RevisionId::new(), DocumentId::new(), vec![],
            "NOTES/ONE.md", b"other document",
        );
        let collision = snapshot(ns.vault_id(), &[old.clone(), tip.clone(), foreign]);
        assert!(preview_verified_s3_existing_content_update(
            &collision, &ns, &base, &intent).is_err());

        let mut tomb = tip.clone();
        tomb.operation = RevisionOperation::Tombstone {
            path: SyncRelativePath::parse("notes/one.md").unwrap(),
        };
        let with_tombstone = snapshot(ns.vault_id(), &[old.clone(), tomb]);
        assert!(preview_verified_s3_existing_content_update(
            &with_tombstone, &ns, &base, &intent).is_err());

        let mut renamed = tip.clone();
        renamed.operation = RevisionOperation::Rename {
            from: SyncRelativePath::parse("notes/previous.md").unwrap(),
            to: SyncRelativePath::parse("notes/one.md").unwrap(),
            blob_hash: intent.expected_content_hash.clone(),
        };
        let with_rename = snapshot(ns.vault_id(), &[old.clone(), renamed]);
        assert!(preview_verified_s3_existing_content_update(
            &with_rename, &ns, &base, &intent).is_err());

        let alternate = content(
            tip.id, tip.document_id, tip.parents.clone(),
            "notes/renamed.md", b"new",
        );
        let path_drift = snapshot(ns.vault_id(), &[old, alternate]);
        assert!(preview_verified_s3_existing_content_update(
            &path_drift, &ns, &base, &intent).is_err());
    }

    #[test]
    fn reject_wrong_hash_or_stale_selected_revision() {
        let (ns, base, intent, old, tip) = accepted();
        let snapshot = snapshot(ns.vault_id(), &[old.clone(), tip.clone()]);
        let mut altered = intent.clone();
        altered.expected_content_hash = blake3::hash(b"forged").to_hex().to_string();
        altered.remote_object_id = ns.immutable_blob_key(
            &altered.expected_content_hash,
        ).unwrap();
        assert!(preview_verified_s3_existing_content_update(
            &snapshot, &ns, &base, &altered).is_err());
        let mut stale = intent.clone();
        stale.revision_id = old.id;
        assert!(preview_verified_s3_existing_content_update(
            &snapshot, &ns, &base, &stale).is_err());
    }
}
