use super::{announce_history, paths_changed, record, NoteHistorySaveFailed};
use crate::app_core::{Core, CoreEvent};
use crate::sync::SqliteCustodyIdentityStore;
use crate::sync::{RegisteredReadAdmission, VaultId};
use crate::files::document::write_file_atomic_impl;
use crate::files::entries::{copy_file_impl, create_binary_file_impl, create_file_impl};
use crate::files::error::FileCommandError;
use crate::files::models::FileWriteResult;
use crate::history::{NoteWrite, Source};
use crate::search::paths::{is_markdown, is_note};
use std::path::{Component, Path};
use std::fs;
use std::time::SystemTime;

pub fn write(
    core: &Core,
    path: &Path,
    content: &str,
    expected_hash: Option<&str>,
    source: Source,
    version_name: Option<&str>,
) -> Result<FileWriteResult, FileCommandError> {
    let (before, result) = {
        let _guard = core.file_mutation_guard_for_write()?;
        let before = core.history.disk_before(path, expected_hash);
        let result = write_file_atomic_impl(path, content, expected_hash)?;
        (before, result)
    };
    let write = NoteWrite { disk_before: before, text: content, hash: &result.hash, source, name: version_name };
    record(core, path, write);
    if is_markdown(path) {
        core.search.notify_content_path(path.to_path_buf());
    }
    Ok(result)
}

/// Reject paths that are not pre-existing, plain Markdown files inside the
/// physically registered Core workspace. This is a conservative pathname
/// check, not an OS-handle-pinned guarantee against hostile same-user swaps.
fn checked_registered_note(root: &Path, path: &Path) -> Result<(), FileCommandError> {
    let denied = || FileCommandError::Task {
        message: "Guarded File Gate refused an unregistered or aliased note path".into(),
    };
    if !path.is_absolute() || !is_markdown(path) {
        return Err(denied());
    }
    let relative = path.strip_prefix(root).map_err(|_| denied())?;
    let components: Vec<_> = relative.components().collect();
    if components.is_empty()
        || components.iter().any(|c| !matches!(c, Component::Normal(_)))
        // NTFS may resolve .AQUILUM to the internal .aquilum history root;
        // block casefold aliases on every OS, not just exact spelling.
        || components[0].as_os_str().to_string_lossy()
            .eq_ignore_ascii_case(crate::history::AQUILUM_FOLDER)
    {
        return Err(denied());
    }
    let mut current = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        current.push(component.as_os_str());
        let meta = fs::symlink_metadata(&current).map_err(|_| denied())?;
        #[cfg(windows)]
        let reparse = {
            use std::os::windows::fs::MetadataExt;
            meta.file_attributes() & 0x0000_0400 != 0
        };
        #[cfg(not(windows))]
        let reparse = false;
        if meta.file_type().is_symlink() || reparse
            || (index + 1 == components.len() && !meta.is_file())
            || (index + 1 != components.len() && !meta.is_dir())
        {
            return Err(denied());
        }
    }
    Ok(())
}

/// SYNC-04g-3c — explicit, opt-in normal Markdown write with one existing
/// external OS lock held across BOTH native File Gate and HistoryService.
/// The lock scope and physical root must already have a durable app-private
/// identity receipt; the method never creates or repairs that registration.
///
/// Synchronous host events/search notifications are deliberately sent AFTER
/// releasing the OS lock: callbacks must not recursively acquire the same
/// non-reentrant custody handle. A postflight refusal means bytes or history
/// may have changed and must never be silently retried as a fresh save.
///
/// This does not enable protection for unrelated ordinary Akula entrypoints.
/// Caller must explicitly choose this method; UI autosave is not yet wired.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_with_registered_custody(
    core: &Core,
    admission: &RegisteredReadAdmission<'_>,
    custody: &SqliteCustodyIdentityStore,
    vault_id: VaultId,
    custody_scope: &Path,
    path: &Path,
    content: &str,
    expected_hash: Option<&str>,
    source: Source,
    version_name: Option<&str>,
) -> Result<FileWriteResult, FileCommandError> {
    let denied = || FileCommandError::Task {
        message: "Guarded File Gate refused an unbound workspace or OS-lock custody".into(),
    };
    if !admission.belongs_to_core(core) {
        return Err(denied());
    }
    admission.recheck_registered_root().map_err(|_| denied())?;
    checked_registered_note(admission.registered_root_path(), path)?;

    let mut entered = false;
    let observed = custody.with_cooperative_writer_lifetime(
        admission, vault_id, custody_scope, || {
            entered = true;
            admission.recheck_registered_root().map_err(|_| denied())?;
            checked_registered_note(admission.registered_root_path(), path)?;
            // Preserve lock order: held external OS lock → process-global
            // File Gate mutex → release mutex → HistoryService files mutex.
            let (before, result) = {
                let _guard = core.file_mutation_guard_for_write()?;
                let before = core.history.disk_before(path, expected_hash);
                let result = write_file_atomic_impl(path, content, expected_hash)?;
                (before, result)
            };
            let write = NoteWrite {
                disk_before: before, text: content, hash: &result.hash,
                source, name: version_name,
            };
            // History failure is not a rollback of the successfully published
            // note. Return it as a warning to report AFTER releasing OS lock.
            let history = core.history.try_saved(&|| core.known_roots(), path, write);
            Ok::<_, FileCommandError>((result, history))
        },
    );

    let (result, history) = match observed {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => return Err(error),
        Err(_) if entered => return Err(FileCommandError::Task {
            message: "Guarded File Gate postflight uncertain: note/history may be written; manual review required; do not retry automatically".into(),
        }),
        Err(_) => return Err(denied()),
    };

    // Never emit synchronous user/editor/MCP callbacks under the OS lock.
    match history {
        Ok(true) => announce_history(core, path),
        Ok(false) => {}
        Err(error) => {
            eprintln!("[aquilum:history] {}: {error}", path.display());
            core.emit(CoreEvent::NoteHistorySaveFailed(NoteHistorySaveFailed {
                path: path.to_string_lossy().into_owned(),
                action: error.action.to_owned(),
                message: error.message,
            }));
        }
    }
    core.search.notify_content_path(path.to_path_buf());
    Ok(result)
}

pub fn create(
    core: &Core,
    path: &Path,
    content: &str,
    source: Source,
    version_name: Option<&str>,
) -> Result<FileWriteResult, FileCommandError> {
    let result = {
        let _guard = core.file_mutation_guard_for_write()?;
        create_file_impl(path, content)?
    };
    if is_note(path) {
        let write = NoteWrite {
            disk_before: Some((String::new(), SystemTime::now())),
            text: content,
            hash: &result.hash,
            source,
            name: version_name,
        };
        record(core, path, write);
    }
    paths_changed(core, vec![path.to_path_buf()]);
    Ok(result)
}

pub fn create_binary(
    core: &Core,
    path: &Path,
    bytes: &[u8],
) -> Result<FileWriteResult, FileCommandError> {
    let result = {
        let _guard = core.file_mutation_guard_for_write()?;
        create_binary_file_impl(path, bytes)?
    };
    paths_changed(core, vec![path.to_path_buf()]);
    Ok(result)
}

pub fn copy(core: &Core, from: &Path, to: &Path) -> Result<(), FileCommandError> {
    {
        let _guard = core.file_mutation_guard_for_write()?;
        copy_file_impl(from, to)?;
    }
    paths_changed(core, vec![to.to_path_buf()]);
    Ok(())
}

pub fn name_version(
    core: &Core,
    path: &Path,
    version: Option<&str>,
    name: &str,
) -> Option<String> {
    let named = core.history.name(&|| core.known_roots(), path, version, name);
    if named.is_some() {
        announce_history(core, path);
    }
    named
}

/// Create a workspace folder under the same managed File Gate guard as note
/// creation. Do not add parent directories or overwrite an existing path.
/// This serializes ordinary Akula folder creation; it is not a sync-write API.
pub fn create_folder(core: &Core, path: &Path) -> Result<(), FileCommandError> {
    use std::io::ErrorKind;
    let _guard = core.file_mutation_guard_for_write()?;
    match std::fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists =>
            Err(FileCommandError::AlreadyExists {
                path: path.to_string_lossy().into_owned(),
            }),
        Err(error) => Err(error.into()),
    }
}

/// Create a missing template directory tree under the Core File Gate.
/// Keep create_dir_all semantics for native starter templates: parents can be
/// created, an existing directory is accepted, and an existing file fails.
/// This is process-local serialization only, not root confinement or a
/// handle-relative sync mutation authority.
pub fn ensure_folder_tree(core: &Core, path: &Path) -> Result<(), FileCommandError> {
    let _guard = core.file_mutation_guard_for_write()?;
    std::fs::create_dir_all(path)?;
    Ok(())
}

#[cfg(test)]
mod folder_tests {
    use super::*;
    use crate::app_core::{Core, CoreEvent};
    use std::fs;
    use std::sync::{mpsc, Arc};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn core_file_gate_serializes_folder_creation_without_clobber() {
        let temp = tempfile::tempdir().unwrap();
        let vault = temp.path().join("vault");
        let app_data = temp.path().join("appdata");
        fs::create_dir(&vault).unwrap();
        fs::create_dir(&app_data).unwrap();
        let core = Core::open(&app_data, Arc::new(|_: CoreEvent| {}));
        let folder = vault.join("Новая папка 😀");

        let held = core.file_mutations.lock().unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let other_core = Arc::clone(&core);
        let target = folder.clone();
        let worker = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            done_tx.send(create_folder(&other_core, &target)).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(
            done_rx.recv_timeout(Duration::from_millis(150)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ), "folder creation bypassed the Core File Gate lock");
        assert!(!folder.exists());
        drop(held);

        done_rx.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
        worker.join().unwrap();
        assert!(folder.is_dir());
        assert!(matches!(
            create_folder(&core, &folder),
            Err(FileCommandError::AlreadyExists { .. })
        ));
    }

    #[test]
    fn core_file_gate_serializes_template_folder_tree_across_core_instances() {
        let temp = tempfile::tempdir().unwrap();
        let vault = temp.path().join("vault");
        let app_a = temp.path().join("app-a");
        let app_b = temp.path().join("app-b");
        for path in [&vault, &app_a, &app_b] {
            fs::create_dir(path).unwrap();
        }
        let first = Core::open(&app_a, Arc::new(|_: CoreEvent| {}));
        let second = Core::open(&app_b, Arc::new(|_: CoreEvent| {}));
        let templates = vault.join("Шаблоны 😀").join("Nested");
        let expected = templates.clone();

        let held = first.file_mutations.lock().unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            done_tx.send(ensure_folder_tree(&second, &templates)).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(
            done_rx.recv_timeout(Duration::from_millis(150)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ), "template folder creation bypassed the process-wide File Gate");
        assert!(!expected.parent().unwrap().exists());
        drop(held);

        done_rx.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
        worker.join().unwrap();
        assert!(expected.is_dir());
        let existing = expected.join("Keep.md");
        fs::write(&existing, b"unchanged").unwrap();
        ensure_folder_tree(&first, &expected).unwrap();
        assert_eq!(fs::read(&existing).unwrap(), b"unchanged");
        assert!(ensure_folder_tree(&first, &existing).is_err());
        assert_eq!(fs::read(&existing).unwrap(), b"unchanged");
    }
}
