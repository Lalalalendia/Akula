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
