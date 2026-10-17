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
const MAX_DISPLAYED_PATHS: usize = 5;
const MAX_JOURNAL_TICKETS_FOR_UI: usize = 1000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeRecoveryOverview {
    /// The operation journal was never initialized in host app-data.
    JournalNotConfigured,
    /// A valid journal has no durable local publication attempts at this time.
    NoLocalPublicationClaims,
    /// Human investigation is needed. These are historical operation names,
    /// not proof those filenames currently exist.
    RequiresReview { total_claims: usize, example_paths: Vec<String> },
    /// Registration, SQLite read, schema, path integrity or size check failed.
    /// Never translate this to "clean" or implicitly repair the database.
    InspectionUnavailable,
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
    let Ok(entries) = journal.local_publication_review_queue() else {
        return InspectionUnavailable;
    };
    if entries.len() > MAX_JOURNAL_TICKETS_FOR_UI {
        return InspectionUnavailable;
    }
    let workspace_id = VaultId::from_uuid(host.registered_workspace_id());
    let selected: Vec<_> = entries.iter()
        .filter(|entry| entry.vault_id == workspace_id).collect();
    if host.recheck_registered_root().is_err() {
        return InspectionUnavailable;
    }
    if selected.is_empty() {
        return NoLocalPublicationClaims;
    }
    let mut example_paths: Vec<String> = selected.iter()
        .take(MAX_DISPLAYED_PATHS)
        .map(|entry| entry.path.as_str().to_string())
        .collect();
    // Deterministic display, no sorting of or truncation of user bytes.
    example_paths.sort();
    RequiresReview { total_claims: selected.len(), example_paths }
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
            example_paths: vec![
                "notes/conflict.md".to_string(),
                "notes/pending.md".to_string(),
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
        journal.connection.execute_batch("PRAGMA user_version = 4;").unwrap();
        assert_eq!(native_recovery_overview(&core, &appdata, &root),
            NativeRecoveryOverview::InspectionUnavailable);
        let version: i64 = journal.connection.query_row(
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
}
