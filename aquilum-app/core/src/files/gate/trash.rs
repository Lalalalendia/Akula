use super::{announce, paths_changed, report_history_move_failure, Relocation};
use crate::app_core::Core;
use crate::files::deletions::deletion_files;
use crate::files::error::FileCommandError;
use crate::files::trash::{ensure_trash_impl, move_to_trash_impl, restore_from_trash_impl};
use crate::search::paths::is_note;
use std::path::{Path, PathBuf};

/// A user-initiated trash-folder creation through the process-wide File Gate.
/// Opening the directory in the OS shell happens only after this guard drops.
/// This is not a remote sync mutation authority or an external-process lock.
pub fn ensure_trash(core: &Core, workspace: &Path) -> Result<String, FileCommandError> {
    let _guard = core.file_mutation_guard_for_write()?;
    ensure_trash_impl(workspace)
}

/// Timed trash retention with short in-process File Gate critical sections.
/// Scans outside the lock, then validates and deletes each candidate under
/// the same interlock used by in-process restore/trash and pinned preimages.
/// No editor, history or SQLite callbacks happen under this lock.
pub fn cleanup_retention(
    core: &Core,
    workspace: &Path,
    retention_days: u32,
) -> Result<u64, FileCommandError> {
    use crate::files::trash::{
        remove_still_empty_trash_directory, remove_still_expired_trash_candidate,
        retention_directory_candidates, retention_expired_candidates,
    };

    if retention_days == 0 {
        return Ok(0);
    }
    let candidates = retention_expired_candidates(workspace, retention_days)?;
    let mut removed = 0;
    for candidate in candidates {
        {
            let _guard = core.file_mutation_guard_for_write()?;
            if remove_still_expired_trash_candidate(workspace, &candidate, retention_days)? {
                removed += 1;
            }
        }
        std::thread::yield_now();
    }
    let directories = retention_directory_candidates(workspace)?;
    for directory in directories {
        {
            let _guard = core.file_mutation_guard_for_write()?;
            remove_still_empty_trash_directory(workspace, &directory)?;
        }
        std::thread::yield_now();
    }
    Ok(removed)
}

pub fn trash(
    core: &Core,
    workspace: &Path,
    path: &Path,
) -> Result<PathBuf, FileCommandError> {
    core.documents.moving(core, path, || trash_now(core, workspace, path))
}

fn trash_now(core: &Core, workspace: &Path, path: &Path) -> Result<PathBuf, FileCommandError> {
    let moved = {
        let _guard = core.file_mutation_guard_for_write()?;
        move_to_trash_impl(workspace, path)?
    };
    let notes = note_moves(&moved.files);
    let history = &core.history;
    for (note, trashed_at) in &notes {
        if let Err(error) = history.try_trashed(&|| core.known_roots(), note, trashed_at) {
            report_history_move_failure(core, note, trashed_at, &error);
        }
    }
    announce(core, removed(notes.iter().map(|(note, _)| note)));
    paths_changed(core, vec![path.to_path_buf(), moved.root.clone()]);
    Ok(moved.root)
}

pub fn restore(
    core: &Core,
    workspace: &Path,
    trashed: &Path,
) -> Result<PathBuf, FileCommandError> {
    let mut roots = restore_all(core, workspace, &[trashed.to_path_buf()])?;
    Ok(roots.remove(0))
}

pub fn restore_deletion(
    core: &Core,
    workspace: &Path,
    id: &str,
) -> Result<(), FileCommandError> {
    restore_all(core, workspace, &deletion_files(workspace, id)).map(|_| ())
}

fn restore_all(
    core: &Core,
    workspace: &Path,
    trashed: &[PathBuf],
) -> Result<Vec<PathBuf>, FileCommandError> {
    let history = &core.history;
    let mut roots = Vec::new();
    let mut failure = None;
    for path in trashed {
        let attempt = {
            let _guard = core.file_mutation_guard_for_write()?;
            restore_from_trash_impl(workspace, path)
        };
        match attempt {
            Ok(restored) => {
                for (trashed_note, back) in note_moves(&restored.files) {
                    if let Err(error) = history.try_restored(&|| core.known_roots(), &trashed_note, &back) {
                        report_history_move_failure(core, &trashed_note, &back, &error);
                    }
                }
                roots.push(restored.root);
            }
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
    }
    paths_changed(core, trashed.iter().chain(&roots).cloned().collect());
    failure.map_or(Ok(roots), Err)
}

fn note_moves(files: &[(PathBuf, PathBuf)]) -> Vec<(PathBuf, PathBuf)> {
    files
        .iter()
        .filter(|(from, _)| is_note(from))
        .cloned()
        .collect()
}

pub(super) fn removed<'a>(notes: impl Iterator<Item = &'a PathBuf>) -> Relocation {
    Relocation {
        moves: Vec::new(),
        removed: notes.map(|note| note.to_string_lossy().into_owned()).collect(),
    }
}

#[cfg(test)]
mod ensure_trash_gate_tests {
    use super::*;
    use crate::app_core::{Core, CoreEvent};
    use std::fs;
    use std::sync::{mpsc, Arc};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn core_file_gate_serializes_trash_creation_between_core_instances() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("Workspace 😀");
        let app_a = temp.path().join("app-a");
        let app_b = temp.path().join("app-b");
        for path in [&workspace, &app_a, &app_b] {
            fs::create_dir(path).unwrap();
        }
        let first = Core::open(&app_a, Arc::new(|_: CoreEvent| {}));
        let second = Core::open(&app_b, Arc::new(|_: CoreEvent| {}));
        let held = first.file_mutations.lock().unwrap();

        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            started_tx.send(()).unwrap();
            finished_tx.send(ensure_trash(&second, &workspace)).unwrap();
        });

        started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(
            finished_rx.recv_timeout(Duration::from_millis(150)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ), "trash directory creation bypassed the global Core File Gate");
        assert!(!temp.path().join("Workspace 😀").join(".trash").exists());
        drop(held);

        let path = finished_rx.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
        worker.join().unwrap();
        assert!(Path::new(&path).is_dir());
        assert_eq!(ensure_trash(&first, &temp.path().join("Workspace 😀")).unwrap(), path);
    }
}


#[cfg(test)]
mod retention_gate_tests {
    use super::*;
    use crate::app_core::{Core, CoreEvent};
    use std::fs;
    use std::sync::{mpsc, Arc};
    use std::thread;
    use std::time::{Duration, SystemTime};

    #[test]
    fn core_file_gate_serializes_retention_cleanup_between_core_instances() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("vault");
        let trash = workspace.join(".trash");
        let app_a = temp.path().join("app-a");
        let app_b = temp.path().join("app-b");
        for directory in [&trash, &app_a, &app_b] {
            fs::create_dir_all(directory).unwrap();
        }
        let old_folder = trash.join("Old");
        fs::create_dir(&old_folder).unwrap();
        let old = old_folder.join("Old.md");
        let fresh = trash.join("Fresh.md");
        fs::write(&old, b"old snapshot").unwrap();
        fs::write(&fresh, b"new snapshot").unwrap();
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(40 * 24 * 60 * 60))
            .unwrap();

        let first = Core::open(&app_a, Arc::new(|_: CoreEvent| {}));
        let second = Core::open(&app_b, Arc::new(|_: CoreEvent| {}));
        let held = first.file_mutations.lock().unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let root = workspace.clone();
        let worker = thread::spawn(move || {
            started_tx.send(()).unwrap();
            done_tx.send(cleanup_retention(&second, &root, 30)).unwrap();
        });

        started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(
            done_rx.recv_timeout(Duration::from_millis(150)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ), "retention cleanup bypassed the global File Gate");
        assert_eq!(fs::read(&old).unwrap(), b"old snapshot");
        drop(held);

        assert_eq!(done_rx.recv_timeout(Duration::from_secs(30)).unwrap().unwrap(), 1);
        worker.join().unwrap();
        assert!(!old.exists());
        assert!(!old_folder.exists());
        assert_eq!(fs::read(&fresh).unwrap(), b"new snapshot");
        assert!(trash.is_dir());
        assert_eq!(cleanup_retention(&first, &workspace, 0).unwrap(), 0);
    }
}
