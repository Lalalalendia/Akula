use super::{announce, paths_changed, report_history_move_failure, report_record, NoteMove, Relocation};
use crate::history::service::HistoryMoveFailure;
use crate::app_core::Core;
use crate::files::entries::rename_file_impl;
use crate::files::error::FileCommandError;
use crate::files::models::FileRenameResult;
use crate::files::rename::rename_with_links;
use crate::history::{NoteWrite, Source};
use crate::search::paths::is_note;
use std::path::{Path, PathBuf};
mod admission;
use admission::admit_vault_relocation;
use std::time::SystemTime;
use walkdir::WalkDir;

// Keep host events outside the full primary/history publication lease.
type DeferredMoves = Vec<(PathBuf, PathBuf, HistoryMoveFailure)>;

fn report_deferred_moves(core: &Core, failures: DeferredMoves) {
    for (from, to, error) in failures {
        report_history_move_failure(core, &from, &to, &error);
    }
}

pub fn rename(
    core: &Core,
    from: &Path,
    to: &Path,
) -> Result<FileRenameResult, FileCommandError> {
    core.documents.moving(core, from, || rename_now(core, from, to))
}

fn rename_now(core: &Core, from: &Path, to: &Path) -> Result<FileRenameResult, FileCommandError> {
    let (renamed, relocation, failures, rewritten_history) = {
        // Publication custody must span BOTH the primary rename/link rewrites
        // AND history transfer and snapshots. Activation takes the exclusive
        // epoch before it publishes participant HOLD.
        #[cfg(feature = "sync-s3")]
        let _publication = core.ordinary_note_publication_lease()?;
        let (renamed, relocation, failures) =
            relocating(core, from, to, || rename_with_links(&core.search, from, to))?;
        let mut rewritten_history = Vec::with_capacity(renamed.rewritten.len());
        for rewrite in &renamed.rewritten {
            let write = NoteWrite {
                disk_before: Some((rewrite.before.clone(), SystemTime::now())),
                text: &rewrite.after,
                hash: &rewrite.hash,
                source: Source::Links,
                name: None,
            };
            rewritten_history.push((
                rewrite.path.clone(),
                core.history.try_saved(&|| core.known_roots(), &rewrite.path, write),
            ));
        }
        (renamed, relocation, failures, rewritten_history)
    };
    // No host callback, event, search update or DocumentHub reconciliation
    // while holding the publication epoch or File Gate / HistoryFiles mutex.
    report_deferred_moves(core, failures);
    for (path, outcome) in rewritten_history {
        report_record(core, &path, outcome);
    }
    let rewritten: Vec<PathBuf> = renamed.result.updated_paths.iter().map(PathBuf::from).collect();
    let mut changed = vec![from.to_path_buf(), to.to_path_buf()];
    changed.extend(rewritten.iter().cloned());
    announce(core, relocation);
    paths_changed(core, changed);
    core.documents.reconcile_paths(core, &rewritten);
    Ok(renamed.result)
}

pub fn move_across(core: &Core, from: &Path, to: &Path) -> Result<(), FileCommandError> {
    core.documents.moving(core, from, || move_across_now(core, from, to))
}

fn move_across_now(core: &Core, from: &Path, to: &Path) -> Result<(), FileCommandError> {
    let ((), relocation, failures) = {
        #[cfg(feature = "sync-s3")]
        let _publication = core.ordinary_note_publication_lease()?;
        relocating(core, from, to, || rename_file_impl(from, to))?
    };
    report_deferred_moves(core, failures);
    announce(core, relocation);
    paths_changed(core, vec![from.to_path_buf(), to.to_path_buf()]);
    Ok(())
}

fn relocating<T>(
    core: &Core,
    from: &Path,
    to: &Path,
    run: impl FnOnce() -> Result<T, FileCommandError>,
) -> Result<(T, Relocation, DeferredMoves), FileCommandError> {
    let (done, relocation, registered_roots) = {
        // Lock preflight enumeration, rename and all link rewrites as one
        // in-process filesystem critical section. History and UI callbacks
        // run only after releasing the lock.
        let _guard = core.file_mutation_guard_for_write()?;
        // Deny before the primary file rename, including directory moves.
        // Never acquire the HistoryService mutex while File Gate is held.
        let registered_roots = admit_vault_relocation(core, from, to)?;
        let notes = notes_under(from);
        let done = run()?;
        (done, relocated(&notes, from, to), registered_roots)
    };
    let mut failures = DeferredMoves::new();
    for step in &relocation.moves {
        let from = Path::new(&step.from);
        let to = Path::new(&step.to);
        if let Err(error) = core.history.try_moved(&|| registered_roots.clone(), from, to) {
            failures.push((from.to_path_buf(), to.to_path_buf(), error));
        }
    }
    Ok((done, relocation, failures))
}

pub(super) fn notes_under(path: &Path) -> Vec<PathBuf> {
    WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && is_note(entry.path()))
        .map(|entry| entry.into_path())
        .collect()
}

pub(super) fn relocated(notes: &[PathBuf], from: &Path, to: &Path) -> Relocation {
    let moves = notes
        .iter()
        .filter_map(|note| {
            let target = crate::files::rebase(note, from, to)?;
            Some(NoteMove {
                from: note.to_string_lossy().into_owned(),
                to: target.to_string_lossy().into_owned(),
            })
        })
        .collect();
    Relocation {
        moves,
        removed: Vec::new(),
    }
}

#[cfg(test)]
mod cross_vault_admission_tests {
    use super::*;
    use crate::app_core::CoreEvent;
    use std::fs;
    use std::sync::Arc;

    #[test]
    fn history_cross_vault_registry_failure_does_not_become_unrestricted_admission() {
        let temp = tempfile::tempdir().unwrap();
        let uninitialized_data = temp.path().join("missing-app-data");
        let source = temp.path().join("Before.md");
        let target = temp.path().join("After.md");
        fs::write(&source, b"keep original").unwrap();
        let core = Core::open(&uninitialized_data, Arc::new(|_: CoreEvent| {}));
        assert!(core.ui_state.workspace_roots().is_err());

        let error = admit_vault_relocation(&core, &source, &target).unwrap_err();
        assert!(
            matches!(&error, FileCommandError::Task { .. }),
            "missing registry must fail closed: {error:?}"
        );
        assert_eq!(fs::read(&source).unwrap(), b"keep original");
        assert!(!target.exists());
    }
}
