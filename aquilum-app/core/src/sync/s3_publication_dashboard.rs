//! S3-E2E-01: show a verified, already registered S3 publication claim
//! in a read-only Native diagnostics panel after a cold Core restart.
//!
//! The shared cloud VaultId is NOT the machine-local workspace UUID. The
//! ordinary recovery dashboard deliberately filters by that local UUID and
//! must not silently grant S3 authority by treating the IDs as equivalent.
//! This bridge follows the existing host-owned SQLite S3 profile binding.
//! Nothing here reads network bytes, writes user notes, releases a File Gate,
//! records an operator decision, grants ACK or advances ConfirmedBaseline.

use super::profile_binding::SqliteRegisteredS3Bindings;
use super::recovery_dashboard::{
    guarded_recovery_journal, MAX_DISPLAYED_PATHS, MAX_JOURNAL_TICKETS_FOR_UI,
};
use super::{
    NativeRecoveryExample, NativeRecoveryOverview, RegisteredReadAdmission,
    SqliteSyncJournal,
};
use crate::app_core::Core;
use std::fs;
use std::path::Path;

/// Test/prototype host-owned path, never derived from remote S3 metadata.
pub const S3_PROFILE_BINDINGS_FILENAME: &str = "s3-profiles.sqlite";

/// Display pending *historical* publication attempts from a particular,
/// already registered local workspace's S3 VaultId. It is a diagnostic,
/// NOT a statement that the remote S3 owner was independently authenticated.
///
/// Any missing/corrupt/mismatched profile, noncanonical file, incompatible
/// journal or raced root registration fails closed. Neither this function nor
/// its status is a native durability receipt.
pub fn native_s3_bound_publication_overview(
    core: &Core,
    app_data_dir: &Path,
    registered_vault_root: &Path,
) -> NativeRecoveryOverview {
    use NativeRecoveryOverview::*;

    let Ok(host) = RegisteredReadAdmission::open(core, registered_vault_root) else {
        return InspectionUnavailable;
    };
    let Ok(appdata) = fs::canonicalize(app_data_dir) else {
        return InspectionUnavailable;
    };
    let Ok(vault) = fs::canonicalize(registered_vault_root) else {
        return InspectionUnavailable;
    };
    if appdata.starts_with(&vault) || vault.starts_with(&appdata) {
        return InspectionUnavailable;
    }

    let bindings_path = appdata.join(S3_PROFILE_BINDINGS_FILENAME);
    match fs::symlink_metadata(&bindings_path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return JournalNotConfigured;
        }
        Err(_) => return InspectionUnavailable,
        Ok(_) => {}
    }
    let Ok(bindings) = SqliteRegisteredS3Bindings::open_existing_read_only(
        &host, &bindings_path,
    ) else {
        return InspectionUnavailable;
    };
    let bound = match bindings.load_for_registered(&host) {
        Ok(Some(bound)) => bound,
        Ok(None) => return JournalNotConfigured,
        Err(_) => return InspectionUnavailable,
    };

    // Reuse the exact Native path, symlink/hardlink and registration checks;
    // the old dashboard must remain non-authorizing and backward compatible.
    let journal_path = appdata.join(super::RECOVERY_JOURNAL_FILENAME);
    match fs::symlink_metadata(&journal_path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return JournalNotConfigured;
        }
        Err(_) => return InspectionUnavailable,
        Ok(_) => {}
    }
    let Ok((_, checked_path)) = guarded_recovery_journal(
        core, &appdata, registered_vault_root,
    ) else {
        return InspectionUnavailable;
    };
    let Ok(journal) = SqliteSyncJournal::open_existing_read_only(&checked_path) else {
        return InspectionUnavailable;
    };
    if journal.require_appdata_outside_vault(&vault).is_err()
        || !matches!(journal.existing_replace_attempts_present(), Ok(false))
    {
        return InspectionUnavailable;
    }
    let Ok(count) = journal.local_publication_count() else {
        return InspectionUnavailable;
    };
    if count > MAX_JOURNAL_TICKETS_FOR_UI as u64 {
        return InspectionUnavailable;
    }
    let Ok(entries) = journal.local_publication_review_queue() else {
        return InspectionUnavailable;
    };
    if entries.len() as u64 != count {
        return InspectionUnavailable;
    }

    let mut selected = Vec::new();
    for entry in entries.iter().filter(|e| e.vault_id == bound.vault_id) {
        let Ok(Some(record)) = journal.get(entry.operation_id) else {
            return InspectionUnavailable;
        };
        // The registered profile name is part of the durable operation.
        // Matching a shared VaultId alone must never expose another scope's
        // path, or be mistaken for an authenticated provider admission.
        if record.intent.profile_id != bound.profile_id {
            return InspectionUnavailable;
        }
        selected.push(entry);
    }

    if host.recheck_registered_root().is_err()
        || !matches!(journal.existing_replace_attempts_present(), Ok(false))
    {
        return InspectionUnavailable;
    }
    if selected.is_empty() {
        return NoLocalPublicationClaims;
    }
    let mut examples: Vec<_> = selected.iter().take(MAX_DISPLAYED_PATHS)
        .map(|entry| NativeRecoveryExample {
            operation_id: entry.operation_id.to_string(),
            historical_path: entry.path.as_str().to_string(),
        }).collect();
    examples.sort_by(|a, b| a.historical_path.cmp(&b.historical_path));
    RequiresReview { total_claims: selected.len(), examples }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{
        DocumentId, JournalAction, JournalPhase, OperationId, OperationIntent,
        RevisionId, S3ConnectionProfile, S3HeadPointerV1, S3Keyspace,
        S3ManifestV1, S3VerifiedHead, SyncRelativePath, VaultId,
    };
    use crate::app_core::CoreEvent;
    use std::sync::Arc;

    fn register(
        root: &Path,
        appdata: &Path,
    ) -> Arc<Core> {
        fs::create_dir_all(root).unwrap();
        fs::create_dir_all(appdata).unwrap();
        let core = Core::open(appdata, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
        core
    }

    fn bind(
        core: &Core, root: &Path, appdata: &Path,
    ) -> (VaultId, String) {
        let keyspace = S3Keyspace::new("akula/s3-e2e01-review", VaultId::new()).unwrap();
        let profile = S3ConnectionProfile {
            bucket: "s3-e2e01-review-test".into(),
            region: "us-east-1".into(),
            endpoint: Some("http://127.0.0.1:9000".into()),
            force_path_style: true,
        };
        let manifest = S3ManifestV1::new(
            &keyspace, OperationId::new(), 1, None, vec![],
        ).unwrap();
        let pointer = S3HeadPointerV1::new(
            &keyspace, 1, manifest.manifest_id,
            &manifest.digest(&keyspace).unwrap(), None,
        ).unwrap();
        let head = S3VerifiedHead {
            pointer, manifest, etag: "\"bounded-fixture-etag\"".into(),
        };
        let admission = RegisteredReadAdmission::open(core, root).unwrap();
        let mut bindings = SqliteRegisteredS3Bindings::open(
            &admission, &appdata.join(S3_PROFILE_BINDINGS_FILENAME),
        ).unwrap();
        let bound = bindings.bind_existing_verified_s3(
            &admission, "s3-receiver-profile", &profile, &keyspace, &head,
        ).unwrap();
        (bound.vault_id, bound.profile_id)
    }

    fn claim(
        journal: &mut SqliteSyncJournal,
        vault: VaultId,
        profile: &str,
        filename: &str,
    ) -> OperationId {
        let bytes = b"# S3-E2E-01: cold receiver review";
        let id = OperationId::new();
        let intent = OperationIntent {
            operation_id: id,
            vault_id: vault,
            document_id: DocumentId::new(),
            profile_id: profile.into(),
            revision_id: RevisionId::new(),
            action: JournalAction::StageDownload,
            path: SyncRelativePath::parse(filename).unwrap(),
            remote_object_id: format!("blobs/{id}"),
            remote_guard: None,
            expected_content_hash: blake3::hash(bytes).to_hex().to_string(),
            expected_baseline: None,
        };
        journal.begin(intent.clone()).unwrap();
        journal.mark_staged(id, bytes).unwrap();
        journal.record_local_publication_attempt_once(&intent).unwrap();
        id
    }

    #[test]
    fn s3_e2e01_registered_remote_claim_is_visible_only_through_bound_read_only_view() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("receiver");
        let appdata = dir.path().join("host-appdata");
        let core = register(&root, &appdata);
        assert_eq!(
            native_s3_bound_publication_overview(&core, &appdata, &root),
            NativeRecoveryOverview::JournalNotConfigured,
        );
        assert!(!appdata.join(S3_PROFILE_BINDINGS_FILENAME).exists());
        let (shared_vault, profile) = bind(&core, &root, &appdata);
        let local_id = core.ui_state.list_workspaces().unwrap()[0].id;
        assert_ne!(shared_vault.as_uuid(), local_id);
        let journal_path = appdata.join(super::super::RECOVERY_JOURNAL_FILENAME);
        let mut journal = SqliteSyncJournal::open(&journal_path).unwrap();
        let op = claim(&mut journal, shared_vault, &profile, "notes/from-cloud.md");
        assert_eq!(journal.get(op).unwrap().unwrap().phase, JournalPhase::Staged);
        assert!(journal.get(op).unwrap().unwrap().intent.expected_baseline.is_none());

        // The old local-ID dashboard (correctly) cannot match remote VaultId.
        assert_eq!(
            super::super::native_recovery_overview(&core, &appdata, &root),
            NativeRecoveryOverview::NoLocalPublicationClaims,
        );
        let expected = NativeRecoveryOverview::RequiresReview {
            total_claims: 1,
            examples: vec![NativeRecoveryExample {
                operation_id: op.to_string(),
                historical_path: "notes/from-cloud.md".into(),
            }],
        };
        assert_eq!(
            native_s3_bound_publication_overview(&core, &appdata, &root),
            expected,
        );
        assert!(journal.baseline(shared_vault, journal.get(op).unwrap().unwrap()
            .intent.document_id).unwrap().is_none());
        drop(journal);
        drop(core);
        let reopened = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        assert_eq!(
            native_s3_bound_publication_overview(&reopened, &appdata, &root),
            expected,
        );
        assert!(!appdata.join("s3-e2e01-confirmed-receipt").exists());
    }

    #[test]
    fn s3_e2e01_foreign_scope_and_profile_do_not_leak_claim_or_clear_hold() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("receiver");
        let other_root = dir.path().join("other");
        let appdata = dir.path().join("host-appdata");
        let core = register(&root, &appdata);
        fs::create_dir(&other_root).unwrap();
        core.ui_state.resolve_workspace(other_root.to_str().unwrap(), 2).unwrap();
        let (shared_vault, profile) = bind(&core, &root, &appdata);
        let mut journal = SqliteSyncJournal::open(
            &appdata.join(super::super::RECOVERY_JOURNAL_FILENAME),
        ).unwrap();
        let op = claim(&mut journal, shared_vault, &profile, "notes/protected.md");
        assert_eq!(
            native_s3_bound_publication_overview(&core, &appdata, &other_root),
            NativeRecoveryOverview::JournalNotConfigured,
        );
        let wrong_profile = claim(
            &mut journal, shared_vault, "unknown-profile", "notes/foreign.md",
        );
        assert_eq!(
            native_s3_bound_publication_overview(&core, &appdata, &root),
            NativeRecoveryOverview::InspectionUnavailable,
        );
        assert!(journal.baseline(
            shared_vault, journal.get(op).unwrap().unwrap().intent.document_id,
        ).unwrap().is_none());
        assert!(journal.get(wrong_profile).unwrap().is_some());
    }
}
