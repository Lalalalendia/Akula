//! SYNC-04g-M1d-d: read-only bridge for native Settings recovery overview.
//!
//! Showing this screen never creates/migrates a SQLite DB, never decrypts or
//! exports forensic archive BLOBs and never changes any sync or filesystem
//! state. It is NOT a manual recovery workflow or authorization to retry.
//! Absence of the journal means NOT CONFIGURED, never "all files synced".
use super::host_admission::RegisteredReadAdmission;
use super::journal::SqliteSyncJournal;
use super::VaultId;
use crate::app_core::Core;
use std::fs;
use std::path::Path;

pub const RECOVERY_JOURNAL_FILENAME: &str = "sync-operations.sqlite3";
pub(super) const MAX_DISPLAYED_PATHS: usize = 5;
pub(super) const MAX_JOURNAL_TICKETS_FOR_UI: usize = 1000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeRecoveryOverview {
    /// The operation journal was never initialized in host app-data.
    JournalNotConfigured,
    /// A valid journal has no durable local publication attempts at this time.
    NoLocalPublicationClaims,
    /// Human investigation is needed. These are historical operation names,
    /// not proof those filenames currently exist.
    RequiresReview { total_claims: usize, examples: Vec<NativeRecoveryExample> },
    /// Registration, SQLite read, schema, path integrity or size check failed,
    /// or this additive-only overview cannot represent existing-file attempts.
    /// Never translate this to "clean" or implicitly repair the database.
    InspectionUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeRecoveryExample {
    pub operation_id: String,
    pub historical_path: String,
}

/// An immutable, specific reviewed snapshot. Contains metadata and digests,
/// never the archived Markdown or binary BLOBs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeRecoveryCasePreview {
    pub operation_id: String,
    pub historical_path: String,
    pub assessment: String,
    pub archive_fingerprint: String,
    pub review_count: u32,
    pub latest_event_digest: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeRecoveryDecision {
    KeepBlocked,
    EscalateManualReview,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeRecoveryReviewError {
    NotReady,
    Stale,
    Refused,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeRecoveryReviewReceipt {
    pub sequence: u32,
    pub decision: NativeRecoveryDecision,
}

/// Caller MUST pass the real host-owned app-data directory and an already
/// selected Core-registered vault. The path is not provided by cloud data.
/// This function only opens the pre-existing canonical journal READ ONLY.
pub fn native_recovery_overview(
    core: &Core,
    app_data_dir: &Path,
    registered_vault_root: &Path,
) -> NativeRecoveryOverview {
    use NativeRecoveryOverview::*;
    let Ok(host) = RegisteredReadAdmission::open(core, registered_vault_root) else {
        return InspectionUnavailable;
    };
    if host.recheck_registered_root().is_err() {
        return InspectionUnavailable;
    }
    let (Ok(app_data), Ok(vault)) = (
        fs::canonicalize(app_data_dir),
        fs::canonicalize(registered_vault_root),
    ) else {
        return InspectionUnavailable;
    };
    // Both should be independent storage roots. A data dir nested in vault
    // leaks unencrypted forensic copies into sync; a vault nested in app-data
    // risks treating journal sidecars as sync documents.
    if app_data.starts_with(&vault) || vault.starts_with(&app_data) {
        return InspectionUnavailable;
    }
    let journal_path = app_data.join(RECOVERY_JOURNAL_FILENAME);
    let metadata = match fs::symlink_metadata(&journal_path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound =>
            return JournalNotConfigured,
        Err(_) => return InspectionUnavailable,
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return InspectionUnavailable;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return InspectionUnavailable;
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return InspectionUnavailable;
        }
    }
    let Ok(journal) = SqliteSyncJournal::open_existing_read_only(&journal_path) else {
        return InspectionUnavailable;
    };
    if journal.require_appdata_outside_vault(&vault).is_err() {
        return InspectionUnavailable;
    }
    // V7 existing-file attempts are not additive publication claims. Their
    // remote VaultId is not necessarily the local workspace UUID either.
    // Until a dedicated bound replacement-review UI exists, fail closed for
    // the whole journal rather than hide a claim or expose another vault's
    // paths. This bounded presence read does not load evidence or file bytes.
    if !matches!(journal.existing_replace_attempts_present(), Ok(false)) {
        return InspectionUnavailable;
    }
    let Ok(known_count) = journal.local_publication_count() else {
        return InspectionUnavailable;
    };
    if known_count > MAX_JOURNAL_TICKETS_FOR_UI as u64 {
        return InspectionUnavailable;
    }
    let Ok(entries) = journal.local_publication_review_queue() else {
        return InspectionUnavailable;
    };
    if entries.len() as u64 != known_count {
        return InspectionUnavailable;
    }
    let workspace_id = VaultId::from_uuid(host.registered_workspace_id());
    let selected: Vec<_> = entries.iter()
        .filter(|entry| entry.vault_id == workspace_id).collect();
    if host.recheck_registered_root().is_err() {
        return InspectionUnavailable;
    }
    // Notice a claim committed while the additive list was being read.
    // This screen is still an observation, not an atomic admission receipt.
    if !matches!(journal.existing_replace_attempts_present(), Ok(false)) {
        return InspectionUnavailable;
    }
    if selected.is_empty() {
        return NoLocalPublicationClaims;
    }
    let mut examples: Vec<NativeRecoveryExample> = selected.iter()
        .take(MAX_DISPLAYED_PATHS)
        .map(|entry| NativeRecoveryExample {
            operation_id: entry.operation_id.to_string(),
            historical_path: entry.path.as_str().to_string(),
        })
        .collect();
    examples.sort_by(|a, b| a.historical_path.cmp(&b.historical_path));
    RequiresReview { total_claims: selected.len(), examples }
}


/// An action can only target one of the at most five currently displayed
/// local publication claims; this is an intentionally bounded first UI slice.
/// A frozen selection will be compared with durable evidence once more when
/// confirming, so a stale window never silently appends a new decision.
pub fn native_recovery_case_preview(
    core: &Core,
    app_data_dir: &Path,
    registered_vault_root: &Path,
    operation_id: &str,
) -> Result<NativeRecoveryCasePreview, NativeRecoveryReviewError> {
    let shown = native_recovery_overview(core, app_data_dir, registered_vault_root);
    let NativeRecoveryOverview::RequiresReview { examples, .. } = shown else {
        return Err(NativeRecoveryReviewError::NotReady);
    };
    if !examples.iter().any(|example| example.operation_id == operation_id) {
        return Err(NativeRecoveryReviewError::Refused);
    }
    let operation = operation_id.parse::<super::OperationId>()
        .map_err(|_| NativeRecoveryReviewError::Refused)?;
    let (host, path) = guarded_recovery_journal(core, app_data_dir, registered_vault_root)?;
    let journal = SqliteSyncJournal::open_existing_read_only(&path)
        .map_err(|_| NativeRecoveryReviewError::NotReady)?;
    let snapshot = host.recovery_operator_preview(operation, &journal)
        .map_err(|_| NativeRecoveryReviewError::NotReady)?;
    host.recheck_registered_root().map_err(|_| NativeRecoveryReviewError::Refused)?;
    Ok(NativeRecoveryCasePreview {
        operation_id: snapshot.operation_id.to_string(),
        historical_path: snapshot.path.as_str().to_string(),
        assessment: format!("{:?}", snapshot.assessment),
        archive_fingerprint: snapshot.archive_fingerprint,
        review_count: snapshot.review_count,
        latest_event_digest: snapshot.latest_event_digest,
    })
}

/// The native host is the trusted app, but this is not OS user authentication.
/// This accepts metadata-only decisions, NEVER a delete/quarantine/retry or
/// baseline authorization. The application must show a dedicated modal and
/// invoke this function only after an explicit real click to confirm.
pub fn native_record_recovery_decision(
    core: &Core,
    app_data_dir: &Path,
    registered_vault_root: &Path,
    displayed: &NativeRecoveryCasePreview,
    decision: NativeRecoveryDecision,
    note: &str,
) -> Result<NativeRecoveryReviewReceipt, NativeRecoveryReviewError> {
    let current = native_recovery_case_preview(
        core, app_data_dir, registered_vault_root, &displayed.operation_id,
    )?;
    if &current != displayed {
        return Err(NativeRecoveryReviewError::Stale);
    }
    let (host, path) = guarded_recovery_journal(core, app_data_dir, registered_vault_root)?;
    // READ_WRITE fails when DB is missing; unlike SqliteSyncJournal::open
    // this method cannot initialize or migrate a missing/outdated schema.
    let mut journal = SqliteSyncJournal::open_existing_review_writer(&path)
        .map_err(|_| NativeRecoveryReviewError::NotReady)?;
    let operation = displayed.operation_id.parse()
        .map_err(|_| NativeRecoveryReviewError::Refused)?;
    let relative = super::SyncRelativePath::parse(&displayed.historical_path)
        .map_err(|_| NativeRecoveryReviewError::Refused)?;
    let disposition = match decision {
        NativeRecoveryDecision::KeepBlocked =>
            super::recovery_review::RecoveryReviewDisposition::KeepBlocked,
        NativeRecoveryDecision::EscalateManualReview =>
            super::recovery_review::RecoveryReviewDisposition::EscalateManualReview,
    };
    let recorded = host.record_recovery_operator_review(
        operation,
        &relative,
        &displayed.archive_fingerprint,
        displayed.review_count,
        &displayed.latest_event_digest,
        disposition,
        note,
        &mut journal,
    ).map_err(|_| NativeRecoveryReviewError::Stale)?;
    Ok(NativeRecoveryReviewReceipt {
        sequence: recorded.sequence,
        decision,
    })
}

/// This is not a cross-process hostile-app-data path lock. Caller supplies
/// host-controlled app-data, never remote or plugin-selected paths; UI review
/// is a local non-destructive event, not release authorization.
pub(super) fn guarded_recovery_journal<'a>(
    core: &'a Core,
    app_data_dir: &Path,
    registered_vault_root: &Path,
) -> Result<(RegisteredReadAdmission<'a>, std::path::PathBuf), NativeRecoveryReviewError> {
    use NativeRecoveryReviewError::Refused;
    let host = RegisteredReadAdmission::open(core, registered_vault_root)
        .map_err(|_| Refused)?;
    host.recheck_registered_root().map_err(|_| Refused)?;
    let app_data = fs::canonicalize(app_data_dir).map_err(|_| Refused)?;
    let vault = fs::canonicalize(registered_vault_root).map_err(|_| Refused)?;
    if app_data.starts_with(&vault) || vault.starts_with(&app_data) {
        return Err(Refused);
    }
    let path = app_data.join(RECOVERY_JOURNAL_FILENAME);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| NativeRecoveryReviewError::NotReady)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(Refused);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(Refused);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(Refused);
        }
    }
    host.recheck_registered_root().map_err(|_| Refused)?;
    Ok((host, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::{Core, CoreEvent};
    use crate::sync::{DocumentId, JournalAction, OperationId, OperationIntent, RevisionId,
        SyncRelativePath, VaultId};
    use std::sync::Arc;

    fn fixture() -> (tempfile::TempDir, Arc<Core>, std::path::PathBuf, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let appdata = temp.path().join("private-data");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&appdata).unwrap();
        let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
        (temp, core, root, appdata)
    }

    fn claim(core: &Core, journal: &mut SqliteSyncJournal, name: &str) -> OperationId {
        let id = OperationId::new();
        let vault_id = VaultId::from_uuid(core.ui_state.list_workspaces().unwrap()[0].id);
        let bytes = b"fixture from remote";
        let intent = OperationIntent {
            operation_id: id,
            vault_id,
            document_id: DocumentId::new(),
            profile_id: "test-profile".into(),
            revision_id: RevisionId::new(),
            action: JournalAction::StageDownload,
            path: SyncRelativePath::parse(name).unwrap(),
            remote_object_id: format!("immutable/{id}"),
            remote_guard: None,
            expected_content_hash: blake3::hash(bytes).to_hex().to_string(),
            expected_baseline: None,
        };
        journal.begin(intent.clone()).unwrap();
        journal.mark_staged(id, bytes).unwrap();
        journal.record_local_publication_attempt_once(&intent).unwrap();
        id
    }

    fn m1de_archived_incident(
        core: &Core,
        root: &Path,
        appdata: &Path,
        path: &str,
    ) -> (OperationId, std::path::PathBuf, std::path::PathBuf) {
        fs::create_dir_all(root.join("notes")).unwrap();
        let op = {
            let mut journal = SqliteSyncJournal::open(
                &appdata.join(RECOVERY_JOURNAL_FILENAME)
            ).unwrap();
            let op = claim(core, &mut journal, path);
            let stage = root.join("notes").join(format!(".akula-sync-stage-{op}"));
            fs::write(&stage, b"fixture from remote").unwrap();
            let final_file = root.join(path);
            fs::write(&final_file, b"other user version").unwrap();
            let host = RegisteredReadAdmission::open(core, root).unwrap();
            host.record_local_recovery_evidence(op, &journal).unwrap();
            host.archive_local_recovery_copy(op, &journal).unwrap();
            op
        };
        (op, root.join("notes").join(format!(".akula-sync-stage-{op}")), root.join(path))
    }

    #[test]
    fn m1de_explicit_review_keeps_both_conflicting_files_and_baseline_unconfirmed() {
        let (_temp, core, root, appdata) = fixture();
        let (op, stage, final_file) = m1de_archived_incident(
            &core, &root, &appdata, "notes/competing.md",
        );
        let preview = native_recovery_case_preview(
            &core, &appdata, &root, &op.to_string(),
        ).unwrap();
        assert_eq!(preview.operation_id, op.to_string());
        assert_eq!(preview.review_count, 0);
        assert_eq!(preview.latest_event_digest, "genesis");
        assert_eq!(preview.assessment, "DifferentFinalFile");
        let first = native_record_recovery_decision(
            &core, &appdata, &root, &preview,
            NativeRecoveryDecision::KeepBlocked, "Keep both user versions",
        ).unwrap();
        assert_eq!(first.sequence, 1);
        assert_eq!(first.decision, NativeRecoveryDecision::KeepBlocked);
        assert_eq!(fs::read(&stage).unwrap(), b"fixture from remote");
        assert_eq!(fs::read(&final_file).unwrap(), b"other user version");
        assert_eq!(native_record_recovery_decision(
            &core, &appdata, &root, &preview,
            NativeRecoveryDecision::EscalateManualReview, "Stale second review",
        ), Err(NativeRecoveryReviewError::Stale));
        let next = native_recovery_case_preview(
            &core, &appdata, &root, &op.to_string(),
        ).unwrap();
        assert_eq!(next.review_count, 1);
        assert_ne!(next.latest_event_digest, "genesis");
        let second = native_record_recovery_decision(
            &core, &appdata, &root, &next,
            NativeRecoveryDecision::EscalateManualReview, "Require manual inspection",
        ).unwrap();
        assert_eq!(second.sequence, 2);
        let db = appdata.join(RECOVERY_JOURNAL_FILENAME);
        let j = SqliteSyncJournal::open_existing_read_only(&db).unwrap();
        assert_eq!(j.recovery_review_history(op).unwrap().len(), 2);
        assert_eq!(j.local_publication_review_queue().unwrap().len(), 1);
        let intent = j.get(op).unwrap().unwrap().intent;
        assert!(j.baseline(intent.vault_id, intent.document_id).unwrap().is_none());
        assert_eq!(fs::read(&stage).unwrap(), b"fixture from remote");
        assert_eq!(fs::read(&final_file).unwrap(), b"other user version");
    }

    #[test]
    fn m1de_missing_archived_evidence_never_allows_native_confirmation() {
        let (_temp, core, root, appdata) = fixture();
        let mut journal = SqliteSyncJournal::open(
            &appdata.join(RECOVERY_JOURNAL_FILENAME)
        ).unwrap();
        let op = claim(&core, &mut journal, "notes/missing.md");
        assert_eq!(native_recovery_case_preview(
            &core, &appdata, &root, &op.to_string(),
        ), Err(NativeRecoveryReviewError::NotReady));
        assert_eq!(native_recovery_case_preview(
            &core, &appdata, &root, &OperationId::new().to_string(),
        ), Err(NativeRecoveryReviewError::Refused));
        assert!(journal.recovery_review_snapshot(op).is_err());
        assert_eq!(journal.local_publication_count().unwrap(), 1);
    }

    #[test]
    fn m1de_cross_vault_and_forged_snapshot_cannot_append_review() {
        let (temp, core, root, appdata) = fixture();
        let (op, _stage, _final_file) = m1de_archived_incident(
            &core, &root, &appdata, "notes/integrity.md",
        );
        let preview = native_recovery_case_preview(
            &core, &appdata, &root, &op.to_string(),
        ).unwrap();
        let other = temp.path().join("other-vault");
        fs::create_dir(&other).unwrap();
        core.ui_state.resolve_workspace(other.to_str().unwrap(), 2).unwrap();
        assert!(native_recovery_case_preview(
            &core, &appdata, &other, &op.to_string(),
        ).is_err());
        assert!(native_record_recovery_decision(
            &core, &appdata, &other, &preview,
            NativeRecoveryDecision::KeepBlocked, "Wrong vault",
        ).is_err());
        let mut forged = preview.clone();
        forged.archive_fingerprint = "f".repeat(64);
        assert_eq!(native_record_recovery_decision(
            &core, &appdata, &root, &forged,
            NativeRecoveryDecision::KeepBlocked, "Forged snapshot",
        ), Err(NativeRecoveryReviewError::Stale));
        let j = SqliteSyncJournal::open_existing_read_only(
            &appdata.join(RECOVERY_JOURNAL_FILENAME)
        ).unwrap();
        assert!(j.recovery_review_history(op).unwrap().is_empty());
    }

    #[test]
    fn m1de_review_writer_never_initializes_or_migrates_database() {
        let (_temp, core, root, appdata) = fixture();
        let db = appdata.join(RECOVERY_JOURNAL_FILENAME);
        assert!(SqliteSyncJournal::open_existing_review_writer(&db).is_err());
        assert!(!db.exists());
        let j = SqliteSyncJournal::open(&db).unwrap();
        drop(j);
        let direct = rusqlite::Connection::open(&db).unwrap();
        direct.execute_batch("PRAGMA user_version = 4;").unwrap();
        assert!(SqliteSyncJournal::open_existing_review_writer(&db).is_err());
        let version: i64 = direct.query_row(
            "PRAGMA user_version", [], |r| r.get(0),
        ).unwrap();
        assert_eq!(version, 4);
        assert_eq!(native_recovery_overview(&core, &appdata, &root),
            NativeRecoveryOverview::InspectionUnavailable);
    }

    #[test]
    fn m1dd_settings_never_creates_a_missing_recovery_journal() {
        let (_temp, core, root, appdata) = fixture();
        let db = appdata.join(RECOVERY_JOURNAL_FILENAME);
        assert!(!db.exists());
        assert_eq!(native_recovery_overview(&core, &appdata, &root),
            NativeRecoveryOverview::JournalNotConfigured);
        assert!(!db.exists());
        assert!(!db.with_extension("sqlite3-wal").exists());
        assert!(!db.with_extension("sqlite3-shm").exists());
    }

    #[test]
    fn m1dd_live_registered_vault_displays_only_metadata_from_existing_claims() {
        let (_temp, core, root, appdata) = fixture();
        let db = appdata.join(RECOVERY_JOURNAL_FILENAME);
        let mut journal = SqliteSyncJournal::open(&db).unwrap();
        assert_eq!(native_recovery_overview(&core, &appdata, &root),
            NativeRecoveryOverview::NoLocalPublicationClaims);
        let one = claim(&core, &mut journal, "notes/pending.md");
        let two = claim(&core, &mut journal, "notes/conflict.md");
        let expected = NativeRecoveryOverview::RequiresReview {
            total_claims: 2,
            examples: vec![
                NativeRecoveryExample {
                    operation_id: two.to_string(),
                    historical_path: "notes/conflict.md".to_string(),
                },
                NativeRecoveryExample {
                    operation_id: one.to_string(),
                    historical_path: "notes/pending.md".to_string(),
                },
            ],
        };
        assert_eq!(native_recovery_overview(&core, &appdata, &root), expected);
        // UI inspection must never mutate claim records or fabricate baselines.
        assert!(journal.get(one).unwrap().is_some());
        assert!(journal.get(two).unwrap().is_some());
        assert_eq!(journal.local_publication_review_queue().unwrap().len(), 2);
    }

    #[test]
    fn m1dd_unregistered_vault_and_appdata_inside_vault_fail_closed() {
        let (_temp, core, root, appdata) = fixture();
        let other = root.parent().unwrap().join("unregistered");
        fs::create_dir(&other).unwrap();
        assert_eq!(native_recovery_overview(&core, &appdata, &other),
            NativeRecoveryOverview::InspectionUnavailable);
        assert_eq!(native_recovery_overview(&core, &root, &root),
            NativeRecoveryOverview::InspectionUnavailable);
    }

    #[test]
    fn m1dd_old_schema_is_not_migrated_by_read_only_settings() {
        let (_temp, core, root, appdata) = fixture();
        let db = appdata.join(RECOVERY_JOURNAL_FILENAME);
        let journal = SqliteSyncJournal::open(&db).unwrap();
        drop(journal);
        let direct = rusqlite::Connection::open(&db).unwrap();
        direct.execute_batch("PRAGMA user_version = 4;").unwrap();
        assert_eq!(native_recovery_overview(&core, &appdata, &root),
            NativeRecoveryOverview::InspectionUnavailable);
        let version: i64 = direct.query_row(
            "PRAGMA user_version", [], |r| r.get(0),
        ).unwrap();
        assert_eq!(version, 4);
    }

    #[cfg(unix)]
    #[test]
    fn m1dd_symlink_or_hardlink_recovery_journal_refuses_untrusted_ui() {
        use std::os::unix::fs::symlink;
        let (_temp, core, root, appdata) = fixture();
        let db = appdata.join(RECOVERY_JOURNAL_FILENAME);
        let other = appdata.join("real.sqlite3");
        let journal = SqliteSyncJournal::open(&other).unwrap();
        drop(journal);
        symlink(&other, &db).unwrap();
        assert_eq!(native_recovery_overview(&core, &appdata, &root),
            NativeRecoveryOverview::InspectionUnavailable);
        fs::remove_file(&db).unwrap();
        fs::hard_link(&other, &db).unwrap();
        assert_eq!(native_recovery_overview(&core, &appdata, &root),
            NativeRecoveryOverview::InspectionUnavailable);
    }

    include!("recovery_existing_replace_tests.rs");
}
