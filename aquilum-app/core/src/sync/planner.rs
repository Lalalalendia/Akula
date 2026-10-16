//! Stage 02 pure planning. Never writes a file, remote object, or sync baseline.
//!
//! An observation of "missing" is NOT an authorized delete. Every destructive
//! action must be separately justified by a verified tombstone, local protection,
//! confirmed baseline and a guarded executor (stage 04).

use super::dag::{RevisionDag, RevisionOperation};
use super::{DocumentId, RevisionId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObservedObject {
    /// Immutable record ID plus independently observed BLAKE3 payload bytes.
    Present { revision: RevisionId, content_hash: String },
    /// Explicit event in the immutable DAG. Never implied from a failed listing.
    Tombstone { revision: RevisionId },
    /// Only after a fully verified local scan / remote listing.
    VerifiedAbsent,
    Unknown,
    Inaccessible,
    Excluded,
    Placeholder,
}

impl ObservedObject {
    fn revision(&self) -> Option<RevisionId> {
        match self {
            Self::Present { revision, .. } | Self::Tombstone { revision } => Some(*revision),
            _ => None,
        }
    }

    fn is_untrusted(&self) -> bool {
        matches!(
            self,
            Self::Unknown | Self::Inaccessible | Self::Excluded | Self::Placeholder
        )
    }
}

/// First sync is an explicitly previewed process; never assume bootstrap may
/// upload/download just because one side appears empty.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SafetyPolicy {
    pub allow_initial_upload: bool,
    pub allow_initial_download: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotDiff {
    pub document_id: DocumentId,
    pub confirmed_baseline: Option<RevisionId>,
    pub local: ObservedObject,
    pub remote: ObservedObject,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HoldReason {
    UntrustedObservation,
    UnknownRevision,
    IdentityMismatch,
    HashMismatch,
    BaselineDrift,
    MissingWithoutTombstone,
    InitialPreviewRequired,
    TombstoneNeedsReview,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlannedAction {
    NoChange,
    /// Only upload a new immutable revision; no overwritten remote HEAD.
    UploadImmutable { revision: RevisionId },
    /// Stage, hash and recheck dirty buffers before guarded local application.
    StageDownload { revision: RevisionId },
    /// Preserve all heads and show conflict; not last-write-wins.
    PreserveConflict { local: RevisionId, remote: RevisionId },
    Hold(HoldReason),
}

/// Conservative pure plan. Does not invent an absence from network failures.
/// PlannedAction never contains "delete local" or "delete remote".
pub fn plan_snapshot(
    diff: &SnapshotDiff,
    dag: &RevisionDag,
    policy: SafetyPolicy,
) -> PlannedAction {
    use ObservedObject as O;
    use PlannedAction as A;

    if diff.local.is_untrusted() || diff.remote.is_untrusted() {
        return A::Hold(HoldReason::UntrustedObservation);
    }

    let verify_revision = |observed: &O| -> Result<(), HoldReason> {
        let Some(id) = observed.revision() else {
            return Ok(());
        };
        let node = dag.get(id).ok_or(HoldReason::UnknownRevision)?;
        if node.document_id != diff.document_id {
            return Err(HoldReason::IdentityMismatch);
        }
        match (observed, &node.operation) {
            (
                O::Present { content_hash, .. },
                RevisionOperation::Content { blob_hash, .. }
                | RevisionOperation::Rename { blob_hash, .. },
            ) if content_hash == blob_hash => Ok(()),
            (O::Tombstone { .. }, RevisionOperation::Tombstone { .. }) => Ok(()),
            _ => Err(HoldReason::HashMismatch),
        }
    };
    for observed in [&diff.local, &diff.remote] {
        if let Err(reason) = verify_revision(observed) {
            return A::Hold(reason);
        }
    }
    if let Some(base) = diff.confirmed_baseline {
        let Some(base_node) = dag.get(base) else {
            return A::Hold(HoldReason::UnknownRevision);
        };
        if base_node.document_id != diff.document_id {
            return A::Hold(HoldReason::IdentityMismatch);
        }
        for state in [&diff.local, &diff.remote] {
            if let Some(head) = state.revision() {
                if !dag.is_ancestor(base, head).unwrap_or(false) {
                    return A::Hold(HoldReason::BaselineDrift);
                }
            }
        }
    }

    match (&diff.local, &diff.remote) {
        (O::Present { revision: left, .. }, O::Present { revision: right, .. }) => {
            if left == right {
                return A::NoChange;
            }
            if dag.is_ancestor(*right, *left) == Ok(true) {
                return A::UploadImmutable { revision: *left };
            }
            if dag.is_ancestor(*left, *right) == Ok(true) {
                return A::StageDownload { revision: *right };
            }
            A::PreserveConflict { local: *left, remote: *right }
        }
        (O::Tombstone { revision: left }, O::Tombstone { revision: right }) => {
            if left == right { A::NoChange }
            else { A::PreserveConflict { local: *left, remote: *right } }
        }
        (O::Tombstone { revision: left }, O::Present { revision: right, .. })
        | (O::Present { revision: left, .. }, O::Tombstone { revision: right }) => {
            // Even a causally newer tombstone requires explicit user/policy
            // authorization before turning it into a local or remote removal.
            A::PreserveConflict { local: *left, remote: *right }
        }
        (O::Tombstone { .. }, O::VerifiedAbsent)
        | (O::VerifiedAbsent, O::Tombstone { .. }) => {
            A::Hold(HoldReason::TombstoneNeedsReview)
        }
        (O::VerifiedAbsent, O::VerifiedAbsent) => {
            if diff.confirmed_baseline.is_some() {
                A::Hold(HoldReason::MissingWithoutTombstone)
            } else {
                A::NoChange
            }
        }
        (O::Present { revision, .. }, O::VerifiedAbsent) => {
            if diff.confirmed_baseline.is_some() {
                A::Hold(HoldReason::MissingWithoutTombstone)
            } else if policy.allow_initial_upload {
                A::UploadImmutable { revision: *revision }
            } else {
                A::Hold(HoldReason::InitialPreviewRequired)
            }
        }
        (O::VerifiedAbsent, O::Present { revision, .. }) => {
            if diff.confirmed_baseline.is_some() {
                A::Hold(HoldReason::MissingWithoutTombstone)
            } else if policy.allow_initial_download {
                A::StageDownload { revision: *revision }
            } else {
                A::Hold(HoldReason::InitialPreviewRequired)
            }
        }
        // All untrusted observations returned earlier.
        _ => A::Hold(HoldReason::UntrustedObservation),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{RevisionRecord, SyncRelativePath};

    fn hash(data: &[u8]) -> String {
        blake3::hash(data).to_hex().to_string()
    }
    fn rev(
        dag: &mut RevisionDag,
        doc: DocumentId,
        parents: Vec<RevisionId>,
        data: &[u8],
    ) -> RevisionId {
        let id = RevisionId::new();
        dag.ingest(RevisionRecord {
            id,
            document_id: doc,
            parents,
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse("notes/doc.md").unwrap(),
                blob_hash: hash(data),
            },
        }).unwrap();
        id
    }
    fn present(id: RevisionId, data: &[u8]) -> ObservedObject {
        ObservedObject::Present {
            revision: id,
            content_hash: hash(data),
        }
    }
    fn diff(doc: DocumentId, base: Option<RevisionId>,
        local: ObservedObject, remote: ObservedObject) -> SnapshotDiff {
        SnapshotDiff {
            document_id: doc,
            confirmed_baseline: base,
            local,
            remote,
        }
    }

    #[test]
    fn common_ancestor_plans_only_immutable_upload_or_guarded_download() {
        let mut dag = RevisionDag::new();
        let d = DocumentId::new();
        let root = rev(&mut dag, d, vec![], b"0");
        let later = rev(&mut dag, d, vec![root], b"1");
        let upload = diff(d, Some(root), present(later, b"1"), present(root, b"0"));
        let download = diff(d, Some(root), present(root, b"0"), present(later, b"1"));
        assert_eq!(plan_snapshot(&upload, &dag, SafetyPolicy::default()),
            PlannedAction::UploadImmutable { revision: later });
        assert_eq!(plan_snapshot(&download, &dag, SafetyPolicy::default()),
            PlannedAction::StageDownload { revision: later });
    }

    #[test]
    fn concurrent_edits_and_delete_vs_edit_never_choose_a_winner() {
        let mut dag = RevisionDag::new();
        let d = DocumentId::new();
        let root = rev(&mut dag, d, vec![], b"0");
        let left = rev(&mut dag, d, vec![root], b"left");
        let right = rev(&mut dag, d, vec![root], b"right");
        assert_eq!(plan_snapshot(
            &diff(d, Some(root), present(left, b"left"), present(right, b"right")),
            &dag, SafetyPolicy::default()),
            PlannedAction::PreserveConflict { local: left, remote: right });

        let tombstone = RevisionId::new();
        dag.ingest(RevisionRecord {
            id: tombstone,
            document_id: d,
            parents: vec![root],
            operation: RevisionOperation::Tombstone {
                path: SyncRelativePath::parse("notes/doc.md").unwrap(),
            },
        }).unwrap();
        assert_eq!(plan_snapshot(&diff(d, Some(root),
            ObservedObject::Tombstone { revision: tombstone }, present(right, b"right")),
            &dag, SafetyPolicy::default()),
            PlannedAction::PreserveConflict { local: tombstone, remote: right });
    }

    #[test]
    fn a_missing_remote_with_known_baseline_is_not_a_delete_command() {
        let mut dag = RevisionDag::new();
        let d = DocumentId::new();
        let root = rev(&mut dag, d, vec![], b"original");
        let action = plan_snapshot(&diff(d, Some(root),
            present(root, b"original"), ObservedObject::VerifiedAbsent),
            &dag, SafetyPolicy {allow_initial_upload: true, allow_initial_download: true});
        assert_eq!(action, PlannedAction::Hold(HoldReason::MissingWithoutTombstone));
        assert_eq!(plan_snapshot(&diff(d, Some(root),
            ObservedObject::VerifiedAbsent, ObservedObject::VerifiedAbsent),
            &dag, SafetyPolicy::default()),
            PlannedAction::Hold(HoldReason::MissingWithoutTombstone));
    }

    #[test]
    fn explicit_bootstrap_preview_can_stage_first_upload_or_download() {
        let mut dag = RevisionDag::new();
        let d = DocumentId::new();
        let root = rev(&mut dag, d, vec![], b"text");
        let new_upload = diff(d, None, present(root, b"text"), ObservedObject::VerifiedAbsent);
        let new_download = diff(d, None, ObservedObject::VerifiedAbsent, present(root, b"text"));
        assert_eq!(plan_snapshot(&new_upload, &dag, SafetyPolicy::default()),
            PlannedAction::Hold(HoldReason::InitialPreviewRequired));
        assert_eq!(plan_snapshot(&new_download, &dag, SafetyPolicy::default()),
            PlannedAction::Hold(HoldReason::InitialPreviewRequired));
        assert_eq!(plan_snapshot(&new_upload, &dag,
            SafetyPolicy {allow_initial_upload: true, allow_initial_download: false}),
            PlannedAction::UploadImmutable { revision: root });
        assert_eq!(plan_snapshot(&new_download, &dag,
            SafetyPolicy {allow_initial_upload: false, allow_initial_download: true}),
            PlannedAction::StageDownload { revision: root });
    }

    #[test]
    fn unknown_access_exclusion_and_placeholder_always_hold() {
        let mut dag = RevisionDag::new();
        let d = DocumentId::new();
        let root = rev(&mut dag, d, vec![], b"content");
        for uncertain in [
            ObservedObject::Unknown,
            ObservedObject::Inaccessible,
            ObservedObject::Excluded,
            ObservedObject::Placeholder,
        ] {
            assert_eq!(plan_snapshot(&diff(d, None, present(root, b"content"), uncertain),
                &dag, SafetyPolicy {allow_initial_upload: true, allow_initial_download: true}),
                PlannedAction::Hold(HoldReason::UntrustedObservation));
        }
    }

    #[test]
    fn unknown_or_tampered_revision_must_not_become_a_valid_head() {
        let mut dag = RevisionDag::new();
        let d = DocumentId::new();
        let root = rev(&mut dag, d, vec![], b"trusted");
        let unknown = RevisionId::new();
        assert_eq!(plan_snapshot(&diff(d, None, present(unknown, b"unverified"),
            present(root, b"trusted")), &dag, SafetyPolicy::default()),
            PlannedAction::Hold(HoldReason::UnknownRevision));
        assert_eq!(plan_snapshot(&diff(d, None, present(root, b"altered"),
            present(root, b"trusted")), &dag, SafetyPolicy::default()),
            PlannedAction::Hold(HoldReason::HashMismatch));
    }

    #[test]
    fn different_document_identity_or_stale_baseline_blocks_sync() {
        let mut dag = RevisionDag::new();
        let d1 = DocumentId::new();
        let d2 = DocumentId::new();
        let a = rev(&mut dag, d1, vec![], b"a");
        let b = rev(&mut dag, d2, vec![], b"b");
        assert_eq!(plan_snapshot(&diff(d1, None, present(a, b"a"), present(b, b"b")),
            &dag, SafetyPolicy::default()),
            PlannedAction::Hold(HoldReason::IdentityMismatch));
        assert_eq!(plan_snapshot(&diff(d1, Some(a), present(a, b"a"),
            present(a, b"a")), &dag, SafetyPolicy::default()), PlannedAction::NoChange);
        let other_root = rev(&mut dag, d1, vec![], b"unrelated");
        assert_eq!(plan_snapshot(&diff(d1, Some(a),
            present(other_root, b"unrelated"), present(a, b"a")),
            &dag, SafetyPolicy::default()),
            PlannedAction::Hold(HoldReason::BaselineDrift));
    }

    #[test]
    fn a_known_tombstone_needs_explicit_review_before_any_deletion() {
        let mut dag = RevisionDag::new();
        let d = DocumentId::new();
        let root = rev(&mut dag, d, vec![], b"old");
        let tombstone = RevisionId::new();
        dag.ingest(RevisionRecord {
            id: tombstone,
            document_id: d,
            parents: vec![root],
            operation: RevisionOperation::Tombstone {
                path: SyncRelativePath::parse("notes/doc.md").unwrap(),
            },
        }).unwrap();
        assert_eq!(plan_snapshot(&diff(d, Some(root),
            ObservedObject::Tombstone { revision: tombstone },
            ObservedObject::VerifiedAbsent), &dag, SafetyPolicy::default()),
            PlannedAction::Hold(HoldReason::TombstoneNeedsReview));
    }
}
