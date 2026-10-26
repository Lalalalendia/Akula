//! Host-bound S3 receiver for new top-level files only.
//!
//! The exact existing File Gate owns atomic create-if-absent. A created file
//! is explicitly UNCONFIRMED on Windows: no S3 ACK or durable baseline is
//! minted here, and failures require review rather than blind replay.

use std::{collections::BTreeSet, fs, path::Path};
use super::{AwsS3ProbeClient, S3HeadError};
use super::additive_receive::S3AdditiveReceiveError;
use crate::app_core::Core;
use crate::sync::{
    JournalAction, JournalError, OperationId, OperationIntent,
    RegisteredReadAdmission, RevisionOperation, RevisionRecord, S3ConnectionProfile,
    S3Keyspace, SqliteDagStore, SqliteSyncJournal, SyncRelativePath, VaultPortError,
    RECOVERY_JOURNAL_FILENAME,
};
use crate::sync::filegate_custody_identity::{
    CustodyBindingError, SqliteCustodyIdentityStore,
};
use crate::sync::profile_binding::{
    S3BindingError, SqliteRegisteredS3Bindings,
};
use crate::sync::s3_pair_bootstrap::{verify_invited_genesis, S3PairBootstrapError};

const MAX_NOTE_BYTES: u64 = 1024 * 1024;
const MAX_IMPORT_FILES: usize = 128;

#[derive(Debug)]
pub enum S3PairReceiveError {
    Admission(VaultPortError),
    Remote(S3HeadError),
    Trust(S3PairBootstrapError),
    Profile(S3BindingError),
    Custody(CustodyBindingError),
    Journal(JournalError),
    Cache(crate::sync::DagStoreError),
    Receive(S3AdditiveReceiveError),
    InvalidRevision,
    ForeignScope,
    Io(std::io::Error),
}
impl From<VaultPortError> for S3PairReceiveError {
    fn from(e: VaultPortError) -> Self { Self::Admission(e) }
}
impl From<S3HeadError> for S3PairReceiveError {
    fn from(e: S3HeadError) -> Self { Self::Remote(e) }
}
impl From<S3PairBootstrapError> for S3PairReceiveError {
    fn from(e: S3PairBootstrapError) -> Self { Self::Trust(e) }
}
impl From<S3BindingError> for S3PairReceiveError {
    fn from(e: S3BindingError) -> Self { Self::Profile(e) }
}
impl From<CustodyBindingError> for S3PairReceiveError {
    fn from(e: CustodyBindingError) -> Self { Self::Custody(e) }
}
impl From<JournalError> for S3PairReceiveError {
    fn from(e: JournalError) -> Self { Self::Journal(e) }
}
impl From<crate::sync::DagStoreError> for S3PairReceiveError {
    fn from(e: crate::sync::DagStoreError) -> Self { Self::Cache(e) }
}
impl From<S3AdditiveReceiveError> for S3PairReceiveError {
    fn from(e: S3AdditiveReceiveError) -> Self { Self::Receive(e) }
}
impl From<std::io::Error> for S3PairReceiveError {
    fn from(e: std::io::Error) -> Self { Self::Io(e) }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct S3PairReceiveReport {
    /// Real local files published, but NOT durably ACKed.
    pub created_unconfirmed: usize,
    /// Already-local files with exact verified hash are not a collision.
    pub already_present_identical: usize,
    /// Includes collisions, unsupported nested files and historical changes.
    pub needs_review: usize,
    pub observed_remote_revisions: usize,
}

/// This is a Core-host method, not an unauthenticated generic "save blob"
/// capability. The caller supplies a registered workspace, app-private
/// databases and a PRE-EXISTING OS-trusted external File Gate lock scope.
impl AwsS3ProbeClient {
    pub(crate) async fn receive_new_registered_s3_files(
        &self,
        core: &Core,
        registered_root: &Path,
        appdata: &Path,
        external_custody_scope: &Path,
        profile_id: &str,
        profile: &S3ConnectionProfile,
        ns: &S3Keyspace,
        invited_genesis_fingerprint: &str,
    ) -> Result<S3PairReceiveReport, S3PairReceiveError> {
        if self.bound_profile() != profile || self.bound_keyspace() != ns {
            return Err(S3PairReceiveError::ForeignScope);
        }
        // Verify pinned origin before touching ANY local SQLite DB.
        let head = verify_invited_genesis(self, ns, invited_genesis_fingerprint).await?;
        let all = self.materialize_verified_revision_metadata_store(
            16 * 1024 * 1024,
        ).await?.ok_or(S3PairReceiveError::ForeignScope)?;
        let mut records = Vec::new();
        let mut same_document = BTreeSet::new();
        for (_, json) in all.verified_entries() {
            let record: RevisionRecord = serde_json::from_slice(json)
                .map_err(|_| S3PairReceiveError::InvalidRevision)?;
            records.push(record);
        }
        if records.len() > MAX_IMPORT_FILES {
            return Err(S3PairReceiveError::InvalidRevision);
        }
        let admission = RegisteredReadAdmission::open(core, registered_root)?;
        let mut bindings = SqliteRegisteredS3Bindings::open(
            &admission, &appdata.join("s3-profiles.sqlite"),
        )?;
        if bindings.load_for_registered(&admission)?.is_none() {
            bindings.bind_existing_verified_s3(
                &admission, profile_id, profile, ns, &head,
            )?;
        }
        let mut custody = SqliteCustodyIdentityStore::open(
            &admission, &appdata.join("s3-custody.sqlite"),
        )?;
        custody.bind_registered_s3(&admission, &bindings, external_custody_scope)?;
        let mut journal = SqliteSyncJournal::open(
            &appdata.join(RECOVERY_JOURNAL_FILENAME),
        )?;
        let mut cache = SqliteDagStore::open(&appdata.join("blobs.sqlite"))?;
        let mut report = S3PairReceiveReport {
            observed_remote_revisions: records.len(),
            ..Default::default()
        };
        for record in records {
            let RevisionOperation::Content { path, blob_hash } = &record.operation else {
                report.needs_review += 1;
                continue;
            };
            // v0 intentionally rejects nested paths until a host-owned,
            // crash-safe directory-creation operation has been accepted.
            if !record.parents.is_empty() || path.as_str().contains('/')
                || !same_document.insert(record.document_id)
            {
                report.needs_review += 1;
                continue;
            }
            let target = registered_root.join(path.as_str());
            match fs::symlink_metadata(&target) {
                Ok(_) => {
                    // A device's own newly uploaded note is already present.
                    // Treat it as identical only via registered CLOSED pinned
                    // read; a symlink, dirty note or hash mismatch stays HOLD.
                    let identical = admission.read_closed_preflight(path, MAX_NOTE_BYTES)
                        .is_ok_and(|local| local.content_hash == *blob_hash);
                    if identical {
                        report.already_present_identical += 1;
                    } else {
                        report.needs_review += 1;
                    }
                    continue;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            let remote_key = ns.immutable_blob_key(blob_hash)
                .map_err(|_| S3PairReceiveError::InvalidRevision)?;
            let intent = OperationIntent {
                operation_id: OperationId::new(),
                vault_id: ns.vault_id(),
                document_id: record.document_id,
                profile_id: profile_id.to_owned(),
                revision_id: record.id,
                action: JournalAction::StageDownload,
                path: SyncRelativePath::parse(path.as_str())
                    .map_err(|_| S3PairReceiveError::InvalidRevision)?,
                remote_object_id: remote_key,
                remote_guard: None,
                expected_content_hash: blob_hash.clone(),
                expected_baseline: None,
            };
            // No file bytes are written except by the existing no-replace
            // File Gate. An uncertain post-create result is a HOLD.
            self.receive_single_verified_s3_additive_once(
                &all, &admission, &bindings, &custody, external_custody_scope,
                &mut journal, &mut cache, intent, MAX_NOTE_BYTES,
            ).await?;
            report.created_unconfirmed += 1;
            admission.recheck_registered_root()?;
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_new_file_receive_report_is_not_a_baseline_receipt() {
        let report = S3PairReceiveReport {
            created_unconfirmed: 1, already_present_identical: 0,
            needs_review: 0, observed_remote_revisions: 1,
        };
        assert_ne!(report.created_unconfirmed, report.needs_review);
        assert_eq!(report.observed_remote_revisions, 1);
    }
}
