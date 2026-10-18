//! Single remote Content tip -> OS-guarded additive create, never overwrite.
//! This is an explicit Core-internal entrypoint, not an automatic sync worker.
//! Even successful native publication does NOT confirm a per-document baseline.

use std::collections::BTreeSet;
use std::path::Path;

use super::{
    core_stage::S3CoreStageError, AwsS3ProbeClient,
    S3VerifiedRevisionMetadataStore, MAX_CORE_STAGE_BLOB_BYTES,
};
use crate::sync::{
    JournalError, OperationIntent, RevisionDag, RevisionOperation,
    RevisionRecord, SqliteDagStore, SqliteSyncJournal, VaultPortError,
};
use crate::sync::filegate_custody_identity::SqliteCustodyIdentityStore;
use crate::sync::host_admission::{LocalCreateError, RegisteredReadAdmission};
use crate::sync::local_create::UnconfirmedLocalCreate;
use crate::sync::profile_binding::SqliteRegisteredS3Bindings;

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum S3AdditiveReceiveError {
    InvalidBinding,
    AmbiguousHistory,
    ExistingBaseline,
    ChangedRemoteHead,
    InvalidBudget,
    Admission(VaultPortError),
    Journal(JournalError),
    Stage(S3CoreStageError),
    LocalCreate(LocalCreateError),
}

impl From<JournalError> for S3AdditiveReceiveError {
    fn from(value: JournalError) -> Self { Self::Journal(value) }
}
impl From<VaultPortError> for S3AdditiveReceiveError {
    fn from(value: VaultPortError) -> Self { Self::Admission(value) }
}
impl From<S3CoreStageError> for S3AdditiveReceiveError {
    fn from(value: S3CoreStageError) -> Self { Self::Stage(value) }
}
impl From<LocalCreateError> for S3AdditiveReceiveError {
    fn from(value: LocalCreateError) -> Self { Self::LocalCreate(value) }
}

/// Require a single verified live head, content-only history, and one
/// portable path owner. This never grants a File Gate mutation capability.
/// A missing parent, sibling, tombstone, rename, stale selected revision
/// or cross-document path collision is a HOLD, not last-writer-wins.
fn require_single_additive_tip(
    records: &[RevisionRecord],
    intent: &OperationIntent,
) -> Result<(), S3AdditiveReceiveError> {
    let deny = || S3AdditiveReceiveError::AmbiguousHistory;
    if records.is_empty() { return Err(deny()); }
    // History, trash and VCS housekeeping are host-owned namespaces,
    // including case aliases on Windows and macOS. A remote revision
    // must never create an arbitrary object beneath them.
    if intent.path.as_str().split('/').any(|part| {
        part.eq_ignore_ascii_case(".aquilum")
            || part.eq_ignore_ascii_case(".trash")
            || part.eq_ignore_ascii_case(".git")
    }) {
        return Err(deny());
    }
    let mut dag = RevisionDag::new();
    let mut heads = BTreeSet::new();
    let mut parents = BTreeSet::new();
    let path_key = intent.path.collision_key();
    for record in records {
        dag.ingest(record.clone()).map_err(|_| deny())?;
        if record.document_id == intent.document_id {
            let RevisionOperation::Content { path, .. } = &record.operation else {
                return Err(deny());
            };
            if path != &intent.path { return Err(deny()); }
            heads.insert(record.id);
            parents.extend(record.parents.iter().copied());
        } else {
            let collision = match &record.operation {
                RevisionOperation::Content { path, .. }
                | RevisionOperation::Tombstone { path } => {
                    path.collision_key() == path_key
                }
                RevisionOperation::Rename { from, to, .. } => {
                    from.collision_key() == path_key
                        || to.collision_key() == path_key
                }
            };
            if collision { return Err(deny()); }
        }
    }
    if dag.pending_len() != 0 || dag.active_len() != records.len() {
        return Err(deny());
    }
    for parent in parents { heads.remove(&parent); }
    if heads.len() != 1 || !heads.contains(&intent.revision_id) {
        return Err(deny());
    }
    Ok(())
}

impl AwsS3ProbeClient {
    /// Explicit host-approved additive receiver for a fully verified current
    /// S3 snapshot, never a selective V2 advisory preview.
    ///
    /// The host must already have approved the registered workspace, bound
    /// S3 profile and an external, durable custody lock. After a stable
    /// bounded transfer the existing native File Gate performs the only
    /// mutation: atomic no-replace creation under its one-shot SQLite claim.
    ///
    /// An error after publication is uncertain and must never auto-replay.
    /// Neither journal commit nor remote-anchor advancement happens here.
    #[allow(dead_code)]
    pub(crate) async fn receive_single_verified_s3_additive_once(
        &self,
        verified: &S3VerifiedRevisionMetadataStore,
        admission: &RegisteredReadAdmission<'_>,
        bindings: &SqliteRegisteredS3Bindings,
        custody: &SqliteCustodyIdentityStore,
        custody_scope: &Path,
        journal: &mut SqliteSyncJournal,
        cache: &mut SqliteDagStore,
        intent: OperationIntent,
        max_blob_bytes: u64,
    ) -> Result<UnconfirmedLocalCreate, S3AdditiveReceiveError> {
        if max_blob_bytes == 0 || max_blob_bytes > MAX_CORE_STAGE_BLOB_BYTES {
            return Err(S3AdditiveReceiveError::InvalidBudget);
        }
        admission.recheck_registered_root()?;
        let binding = bindings.load_for_registered(admission)
            .map_err(|_| S3AdditiveReceiveError::InvalidBinding)?
            .ok_or(S3AdditiveReceiveError::InvalidBinding)?;
        if !binding.matches_remote(self.bound_profile(), self.bound_keyspace())
            || binding.profile_id != intent.profile_id
            || binding.vault_id != intent.vault_id
        {
            return Err(S3AdditiveReceiveError::InvalidBinding);
        }
        // Must happen BEFORE the stage operation mutates either SQLite DB.
        // The verified immutable cloud bytes may never be stored inside
        // the registered vault, even if local creation later fails closed.
        journal.require_appdata_outside_vault(admission.registered_root_path())?;
        cache.require_appdata_outside_vault(admission.registered_root_path())
            .map_err(|_| S3AdditiveReceiveError::InvalidBinding)?;
        if journal.baseline(intent.vault_id, intent.document_id)?.is_some() {
            return Err(S3AdditiveReceiveError::ExistingBaseline);
        }
        let inventory = verified.verified_revision_inventory().map_err(|err|
            S3AdditiveReceiveError::Stage(S3CoreStageError::Manifest(err))
        )?;
        if !inventory.complete
            || inventory.vault_id != binding.vault_id
            || !inventory.revisions.contains(&intent.revision_id)
        {
            return Err(S3AdditiveReceiveError::AmbiguousHistory);
        }
        let records: Vec<RevisionRecord> = verified.verified_entries()
            .map(|(_, json)| serde_json::from_slice(json)
                .map_err(|_| S3AdditiveReceiveError::AmbiguousHistory))
            .collect::<Result<_, _>>()?;
        require_single_additive_tip(&records, &intent)?;

        // Never fetch bytes before verifying host binding and remote DAG.
        let first_head = self.core_stage_current_head(verified).await?;
        self.stage_verified_s3_content_for_core(
            verified, journal, cache, intent.clone(), max_blob_bytes,
        ).await?;
        let last_head = self.core_stage_current_head(verified).await?;
        if first_head != last_head {
            return Err(S3AdditiveReceiveError::ChangedRemoteHead);
        }
        // The only native mutation is delegated to the existing pinned,
        // custody-bound File Gate. Missing/dirty/occupied targets HOLD.
        let created = admission.create_staged_if_absent_for_registered_s3(
            intent.operation_id, journal, cache, bindings,
            self.bound_profile(), self.bound_keyspace(), custody, custody_scope,
        )?;
        Ok(created)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{
        DocumentId, JournalAction, OperationId, RevisionId, SyncRelativePath, VaultId,
    };

    fn record(
        id: RevisionId, doc: DocumentId, parents: Vec<RevisionId>, path: &str,
    ) -> RevisionRecord {
        RevisionRecord {
            id, document_id: doc, parents,
            operation: RevisionOperation::Content {
                path: SyncRelativePath::parse(path).unwrap(),
                blob_hash: blake3::hash(b"x").to_hex().to_string(),
            },
        }
    }

    fn proposal(doc: DocumentId, rev: RevisionId) -> OperationIntent {
        OperationIntent {
            operation_id: OperationId::new(),
            vault_id: VaultId::new(),
            document_id: doc,
            profile_id: "approved".into(),
            revision_id: rev,
            action: JournalAction::StageDownload,
            path: SyncRelativePath::parse("notes/only.md").unwrap(),
            remote_object_id: "immutable/blob".into(),
            remote_guard: None,
            expected_content_hash: blake3::hash(b"x").to_hex().to_string(),
            expected_baseline: None,
        }
    }

    #[test]
    fn unique_tip_only_and_no_historical_overwrite() {
        let doc = DocumentId::new();
        let first = RevisionId::new();
        let next = RevisionId::new();
        let p = proposal(doc, next);
        let a = record(first, doc, vec![], "notes/only.md");
        let b = record(next, doc, vec![first], "notes/only.md");
        assert!(require_single_additive_tip(&[a.clone(), b.clone()], &p).is_ok());
        assert!(require_single_additive_tip(
            &[a.clone(), b.clone()], &proposal(doc, first)
        ).is_err());

        let sibling = record(RevisionId::new(), doc, vec![first], "notes/only.md");
        assert!(require_single_additive_tip(
            &[a.clone(), b.clone(), sibling], &p
        ).is_err());

        let mut tombstone = b.clone();
        tombstone.operation = RevisionOperation::Tombstone { path: p.path.clone() };
        assert!(require_single_additive_tip(&[a.clone(), tombstone], &p).is_err());

        let mut rename = b.clone();
        rename.operation = RevisionOperation::Rename {
            from: SyncRelativePath::parse("notes/old.md").unwrap(),
            to: p.path.clone(),
            blob_hash: p.expected_content_hash.clone(),
        };
        assert!(require_single_additive_tip(&[a.clone(), rename], &p).is_err());

        let collision = record(
            RevisionId::new(), DocumentId::new(), vec![], "NOTES/ONLY.md",
        );
        assert!(require_single_additive_tip(
            &[a.clone(), b.clone(), collision], &p
        ).is_err());

        let private_id = RevisionId::new();
        let mut private = proposal(doc, private_id);
        private.path = SyncRelativePath::parse("notes/.AQUILUM/history.md").unwrap();
        assert!(require_single_additive_tip(
            &[record(private_id, doc, vec![], "notes/.AQUILUM/history.md")],
            &private,
        ).is_err());

        let stale_path = record(next, doc, vec![first], "notes/other.md");
        assert!(require_single_additive_tip(&[a.clone(), stale_path], &p).is_err());
        let dangling = record(next, doc, vec![RevisionId::new()], "notes/only.md");
        assert!(require_single_additive_tip(&[dangling], &p).is_err());
    }
}
