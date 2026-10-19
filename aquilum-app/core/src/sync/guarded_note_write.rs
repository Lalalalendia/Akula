//! Opt-in, registered-local-note cooperative writer. Ordinary editor saves
//! are NOT routed here: shared OS locks only serialize participating Cores.
//! This is NOT an S3 overwrite/delete permission or a hostile-writer-proof gate.

use super::filegate_custody_identity::SqliteCustodyIdentityStore;
use super::{filegate_lock_custody, resolve_no_symlinks, RegisteredReadAdmission, SyncRelativePath, VaultId};
use crate::app_core::{Core, CoreEvent};
use crate::history::service::HistoryService;
use crate::files::error::FileCommandError;
use crate::files::gate::{self, CheckedHistoryError, NoteHistoryChanged, NoteHistorySaveFailed};
use crate::files::models::FileWriteResult;
use crate::history::Source;
use crate::search::paths::is_note;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_NOTE_BYTES: usize = 32 * 1024 * 1024;

/// P0g-3c: a non-Clone lexical capability minted ONLY inside the held
/// external OS-custody callback, for the exact registered Core and checked
/// note pathname. It is not an OS lock handle or remote baseline authority.
/// Borrowed Core/path references keep the capability scoped to that writer.
pub(crate) struct HeldCustodyHistoryPermit<'a> {
    core: &'a Core,
    note: &'a Path,
}

impl HeldCustodyHistoryPermit<'_> {
    /// Check before the File Gate primary mutation, not only after the write.
    pub(crate) fn binds_core_and_note(&self, core: &Core, note: &Path) -> bool {
        std::ptr::eq(self.core, core) && self.note == note
    }

    /// Check again inside HistoryService before any history-store mutation.
    pub(crate) fn binds_history_and_note(
        &self,
        history: &HistoryService,
        note: &Path,
    ) -> bool {
        std::ptr::eq(&self.core.history, history) && self.note == note
    }
}

/// Denied means neither preparation nor the primary writer was entered.
/// NeedsReview means durable preparation, bytes or history might have changed,
/// including an OS-lock/custody post-check failure. Never retry automatically.
#[derive(Debug)]
pub(crate) enum GuardedNoteWriteError {
    Denied(String),
    NeedsReview(String),
}

fn deny(why: impl Into<String>) -> FileCommandError {
    FileCommandError::Task { message: why.into() }
}

/// Resolve an EXISTING in-vault note. No create, rename, trash, unguarded
/// symlink/junction traversal, or hardlink alias can be admitted here.
/// These pathname checks do not defeat a malicious same-user race.
fn existing_note(
    root: &Path,
    relative: &SyncRelativePath,
    expected_hash: &str,
    content: &str,
) -> Result<PathBuf, FileCommandError> {
    if content.len() > MAX_NOTE_BYTES
        || expected_hash.len() != 64
        || !expected_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(deny("cooperative note write requires bounded content and a BLAKE3 CAS hash"));
    }
    // HistoryService owns .aquilum below each vault, not the editor or a
    // remote revision. Windows casefolds this name; deny portable case
    // aliases and nested instances BEFORE resolving or opening any file.
    // An immutable path/lock check must never authorize writes into the
    // internal version store or its metadata.
    if relative.as_str().split('/').any(|part|
        part.eq_ignore_ascii_case(crate::history::AQUILUM_FOLDER))
    {
        return Err(deny("cooperative note write refuses the private .aquilum history namespace"));
    }
    let path = resolve_no_symlinks(root, relative)
        .map_err(|error| deny(format!("unsafe registered note path: {error:?}")))?;
    if !is_note(&path) {
        return Err(deny("cooperative write only supports existing Markdown/text notes"));
    }
    filegate_lock_custody::require_no_alias_ancestors(
        path.parent().ok_or_else(|| deny("missing note parent"))?,
    ).map_err(|error| deny(format!("aliased note ancestor: {error:?}")))?;
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(deny("cooperative note target is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(deny("hardlinked cooperative note is not allowed"));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x0000_0400 != 0 {
            return Err(deny("reparse-point cooperative note is not allowed"));
        }
        let file = fs::File::open(&path)?;
        let (_, _, links) = super::pinned_windows::file_identity_with_links(&file)
            .map_err(|error| deny(format!("cannot identify note links: {error:?}")))?;
        if links != 1 {
            return Err(deny("hardlinked cooperative note is not allowed"));
        }
    }
    Ok(path)
}

/// A deliberately explicit internal bridge to the REAL Core File Gate and
/// HistoryService. Lock order:
///   external pinned OS lock -> Core File Gate -> release Core File Gate
///   -> HistoryService files/state -> release HistoryService -> OS lock.
/// The registered Core is taken from admission, never an unrelated Core.
/// This API only permits a CAS update of an existing note; it is not wired
/// to ordinary editor/UI or remote sync overwrite.
#[allow(dead_code)]
pub(crate) fn save_existing_note_under_custody(
    admission: &RegisteredReadAdmission<'_>,
    custody: &SqliteCustodyIdentityStore,
    vault_id: VaultId,
    custody_scope: &Path,
    relative: &SyncRelativePath,
    content: &str,
    expected_hash: &str,
    source: Source,
    version_name: Option<&str>,
) -> Result<FileWriteResult, GuardedNoteWriteError> {
    save_existing_note_with_checks_under_custody(
        admission, custody, vault_id, custody_scope, relative, content,
        expected_hash, source, version_name, || Ok(()), |_, _| Ok(()),
    )
}

/// Single cooperative OS lifetime for preparation, the existing File Gate
/// writer, HistoryService and postflight. `T` (for example, a reservation or
/// one-shot attempt observation) stays alive through postflight, cannot be
/// returned as a writer permit, and is dropped before custody is released.
///
/// Callbacks are trusted synchronous host code, NOT UI/plugin hooks. They
/// must not reacquire custody or issue UI notifications. File Gate keeps its
/// existing lock order and releases its mutex before HistoryService locks.
/// Preparation can commit durable evidence before returning an error: every
/// error after entering it is NeedsReview, even if primary bytes are intact.
/// Postflight runs only after successful primary AND history persistence.
///
/// This internal bridge does not validate a remote DAG, mint a native receipt,
/// change installed-host policy or commit a Sync baseline. Its Ok is only the
/// existing local FileWriteResult, not an S3 apply acknowledgement.
#[allow(dead_code)]
pub(crate) fn save_existing_note_with_checks_under_custody<T>(
    admission: &RegisteredReadAdmission<'_>,
    custody: &SqliteCustodyIdentityStore,
    vault_id: VaultId,
    custody_scope: &Path,
    relative: &SyncRelativePath,
    content: &str,
    expected_hash: &str,
    source: Source,
    version_name: Option<&str>,
    prepare: impl FnOnce() -> Result<T, FileCommandError>,
    postflight: impl FnOnce(&T, &FileWriteResult) -> Result<(), FileCommandError>,
) -> Result<FileWriteResult, GuardedNoteWriteError> {
    save_existing_note_with_prewrite_under_custody(
        admission, custody, vault_id, custody_scope, relative, content,
        expected_hash, source, version_name, prepare, |_| Ok(()), postflight,
    )
}

/// P0g-3a: optional same-inode prepublication check, invoked inside the
/// existing File Gate mutex while the one external OS custody lifetime and
/// any retained DocumentHub reservation are still held. A rejection after
/// entering preparation is irreversibly NeedsReview, never retry/rollback.
/// This is a cooperative check, NOT an OS-level compare-and-swap or receipt.
#[allow(dead_code)]
pub(crate) fn save_existing_note_with_prewrite_under_custody<T>(
    admission: &RegisteredReadAdmission<'_>,
    custody: &SqliteCustodyIdentityStore,
    vault_id: VaultId,
    custody_scope: &Path,
    relative: &SyncRelativePath,
    content: &str,
    expected_hash: &str,
    source: Source,
    version_name: Option<&str>,
    prepare: impl FnOnce() -> Result<T, FileCommandError>,
    prewrite: impl FnOnce(&T) -> Result<(), FileCommandError>,
    postflight: impl FnOnce(&T, &FileWriteResult) -> Result<(), FileCommandError>,
) -> Result<FileWriteResult, GuardedNoteWriteError> {
    let core = admission.registered_core();
    let target = admission.registered_root_path().join(relative.as_str());
    let mut entered = false;
    let mut history_changed = false;
    let outcome = custody.with_cooperative_writer_lifetime(
        admission, vault_id, custody_scope, || {
            let checked = existing_note(
                admission.registered_root_path(), relative, expected_hash, content,
            ).map_err(CheckedHistoryError::Primary)?;
            entered = true;
            let prepared = prepare().map_err(CheckedHistoryError::Primary)?;
            let permit = HeldCustodyHistoryPermit {
                core,
                note: &checked,
            };
            let (result, changed) = gate::write_checked_history_with_prewrite(
                &permit, core, &checked, content, expected_hash, source, version_name,
                || prewrite(&prepared),
            )?;
            history_changed = changed;
            postflight(&prepared, &result).map_err(CheckedHistoryError::Primary)?;
            Ok(result)
        },
    );

    // No callback into the UI/search service while the OS lock is held.
    // Completed history must be announced even if a later postflight fails.
    if entered {
        core.search.notify_content_path(target.clone());
    }
    if history_changed {
        core.emit(CoreEvent::NoteHistoryChanged(NoteHistoryChanged {
            path: target.to_string_lossy().into_owned(),
        }));
    }
    match outcome {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(error)) => {
            if let CheckedHistoryError::History(failure) = &error {
                core.emit(CoreEvent::NoteHistorySaveFailed(NoteHistorySaveFailed {
                    path: target.to_string_lossy().into_owned(),
                    action: failure.action.to_owned(),
                    message: failure.message.clone(),
                }));
            }
            let message = format!("{error:?}");
            if entered {
                Err(GuardedNoteWriteError::NeedsReview(message))
            } else {
                Err(GuardedNoteWriteError::Denied(message))
            }
        }
        Err(error) if entered => Err(GuardedNoteWriteError::NeedsReview(format!("{error:?}"))),
        Err(error) => Err(GuardedNoteWriteError::Denied(format!("{error:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::{Core, CoreEvent};
    use crate::history::service::HistoryFault;
    use crate::sync::filegate_custody_identity::SqliteCustodyIdentityStore;
    use crate::sync::filegate_lock_custody::scoped_leaf;
    use crate::sync::filegate_lock_probe::{PinnedProbeRoot, LOCK_NAME};
    use std::sync::Arc;

    #[test]
    fn guarded_note_write_denies_casefolded_private_history_namespace() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        fs::create_dir_all(&root).unwrap();
        // macOS exposes /var as a symlink to /private/var: compare physical,
        // canonical paths, not two different spellings of the same object.
        let root = fs::canonicalize(root).unwrap();
        let history = root.join(".aquilum").join("history");
        fs::create_dir_all(&history).unwrap();
        let protected = history.join("Protected.md");
        fs::write(&protected, b"immutable-internal-version").unwrap();
        let expected = blake3::hash(b"immutable-internal-version").to_hex().to_string();
        // Exact private path, NTFS case alias, and nested hidden history
        // names must be rejected BEFORE any protected file is opened.
        for relative in [
            ".aquilum/history/Protected.md",
            ".AQUILUM/history/Protected.md",
            ".AqUiLuM/history/Protected.md",
            "notes/.aquilum/Protected.md",
        ] {
            let path = SyncRelativePath::parse(relative).unwrap();
            assert!(existing_note(&root, &path, &expected, "forbidden").is_err(),
                "{relative} was accepted as an editable note");
            assert_eq!(fs::read(&protected).unwrap(), b"immutable-internal-version");
        }
        let note = root.join("Normal.md");
        fs::write(&note, b"normal user note").unwrap();
        let expected = blake3::hash(b"normal user note").to_hex().to_string();
        assert_eq!(
            existing_note(&root, &SyncRelativePath::parse("Normal.md").unwrap(),
                &expected, "ordinary edit").unwrap(),
            note,
            "deny the private namespace without blocking ordinary notes",
        );
    }

    #[test]
    fn guarded_note_write_delivers_history_event_only_after_os_unlock() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("appdata");
        let selected_root = temp.path().join("vault");
        fs::create_dir_all(&selected_root).unwrap();
        fs::create_dir_all(&data).unwrap();
        let root = crate::search::paths::canonical_path(&selected_root);
        let note = root.join("Guarded.md");
        fs::write(&note, b"old note").unwrap();
        let physical = PinnedProbeRoot::open(&root).unwrap().identity();
        let scope = temp.path().join("external").join(scoped_leaf(physical));
        fs::create_dir_all(&scope).unwrap();
        let lock = scope.join(LOCK_NAME);
        fs::write(&lock, b"existing cooperative lock").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&scope, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
        }

        // Custody policy rejects aliased ancestors such as macOS /var.
        // Use the real scope path in both app-private binding and OS checks.
        #[cfg(unix)]
        let scope = fs::canonicalize(scope).unwrap();

        let seen = Arc::new(AtomicBool::new(false));
        let seen_from_callback = Arc::clone(&seen);
        let scope_from_callback = scope.clone();
        let core = Core::open(&data, Arc::new(move |event: CoreEvent| {
            if matches!(event, CoreEvent::NoteHistoryChanged(_)) {
                // A synchronous native host listener must be allowed to
                // start another custody operation without reentrant deadlock.
                let pinned = PinnedProbeRoot::open(&scope_from_callback).unwrap();
                let held = pinned.try_lock().unwrap();
                assert!(held.is_some(), "history event emitted under the held OS lock");
                seen_from_callback.store(true, Ordering::SeqCst);
            }
        }));
        core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
        let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut custody = SqliteCustodyIdentityStore::open(
            &admission, &data.join("custody.sqlite"),
        ).unwrap();
        let vault = VaultId::new();
        custody.bind_observed(&admission, vault, &scope).unwrap();

        let content = "changed guarded note and history ".repeat(100);
        let receipt = save_existing_note_under_custody(
            &admission, &custody, vault, &scope,
            &SyncRelativePath::parse("Guarded.md").unwrap(),
            &content, &blake3::hash(b"old note").to_hex().to_string(),
            Source::Me, Some("guarded-saved-version"),
        );
        assert!(receipt.is_ok(), "guarded CAS publication failed: {receipt:?}");
        assert_eq!(fs::read_to_string(&note).unwrap(), content);
        assert!(core.history.page(&|| vec![root.clone()], &note, 0, 8).total > 0);
        assert!(seen.load(Ordering::SeqCst),
            "HistoryService must publish its event after OS custody is released");
        assert_eq!(fs::read(&lock).unwrap(), b"existing cooperative lock");
    }

    #[test]
    fn guarded_note_write_requires_receipt_and_reports_history_failure_as_uncertain() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let data = temp.path().join("appdata");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&data).unwrap();
        let root = crate::search::paths::canonical_path(&root);
        let core = Core::open(&data, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
        let note = root.join("Shared.md");
        fs::write(&note, b"original").unwrap();

        let physical = PinnedProbeRoot::open(&root).unwrap().identity();
        let scope = temp.path().join("external").join(scoped_leaf(physical));
        fs::create_dir_all(&scope).unwrap();
        let lock = scope.join(LOCK_NAME);
        fs::write(&lock, b"existing cooperative lock").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&scope, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
        }

        // Custody policy rejects aliased ancestors such as macOS /var.
        // Use the real scope path in both app-private binding and OS checks.
        #[cfg(unix)]
        let scope = fs::canonicalize(scope).unwrap();

        let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut custody = SqliteCustodyIdentityStore::open(
            &admission, &data.join("custody.sqlite"),
        ).unwrap();
        let vault = VaultId::new();
        let relative = SyncRelativePath::parse("Shared.md").unwrap();
        let before = blake3::hash(b"original").to_hex().to_string();
        let content = "changed note text ".repeat(90);

        let denied = save_existing_note_under_custody(
            &admission, &custody, vault, &scope, &relative,
            &content, &before, Source::Me, Some("test"),
        );
        assert!(matches!(denied, Err(GuardedNoteWriteError::Denied(_))));
        assert_eq!(fs::read(&note).unwrap(), b"original");
        custody.bind_observed(&admission, vault, &scope).unwrap();

        // A failed *history* persistence is not a successful write ACK:
        // the primary bytes may already be visible and must be reviewed.
        core.history.fail_once(HistoryFault::CreateVersion);
        let result = save_existing_note_under_custody(
            &admission, &custody, vault, &scope, &relative,
            &content, &before, Source::Me, Some("test"),
        );
        assert!(matches!(result, Err(GuardedNoteWriteError::NeedsReview(_))));
        assert_eq!(fs::read_to_string(&note).unwrap(), content);
        assert_eq!(fs::read(&lock).unwrap(), b"existing cooperative lock");
    }

    #[cfg(unix)]
    #[test]
    fn guarded_note_write_rejects_symlink_and_hardlink_before_mutation() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        fs::create_dir(&root).unwrap();
        let original = root.join("Original.md");
        let outside = temp.path().join("Outside.md");
        fs::write(&outside, b"original").unwrap();
        symlink(&outside, root.join("Linked.md")).unwrap();
        fs::hard_link(&outside, &original).unwrap();
        let hash = blake3::hash(b"original").to_hex().to_string();
        let content = "must not be written";
        for name in ["Linked.md", "Original.md"] {
            let checked = existing_note(
                &root, &SyncRelativePath::parse(name).unwrap(), &hash, content,
            );
            assert!(checked.is_err(), "{name}");
        }
        assert_eq!(fs::read(&outside).unwrap(), b"original");
    }

    #[path = "guarded_note_transaction_tests.rs"]
    mod transaction;
}
