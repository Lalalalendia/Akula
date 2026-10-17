use super::{announce, paths_changed, record, report_history_move_failure, NoteMove, Relocation};
use crate::app_core::Core;
use crate::files::entries::rename_file_impl;
use crate::files::error::FileCommandError;
use crate::files::models::FileRenameResult;
use crate::files::rename::rename_with_links;
use crate::history::{NoteWrite, Source};
use crate::search::paths::{canonical_path, identity, is_note, strip_root};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

pub fn rename(
    core: &Core,
    from: &Path,
    to: &Path,
) -> Result<FileRenameResult, FileCommandError> {
    core.documents.moving(core, from, || rename_now(core, from, to))
}

fn rename_now(core: &Core, from: &Path, to: &Path) -> Result<FileRenameResult, FileCommandError> {
    let (renamed, relocation) = relocating(core, from, to, || rename_with_links(&core.search, from, to))?;
    for rewrite in &renamed.rewritten {
        let write = NoteWrite {
            disk_before: Some((rewrite.before.clone(), SystemTime::now())),
            text: &rewrite.after,
            hash: &rewrite.hash,
            source: Source::Links,
            name: None,
        };
        record(core, &rewrite.path, write);
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
    let ((), relocation) = relocating(core, from, to, || rename_file_impl(from, to))?;
    announce(core, relocation);
    paths_changed(core, vec![from.to_path_buf(), to.to_path_buf()]);
    Ok(())
}

/// Select a registered workspace conservatively. Aliases resolved by the
/// filesystem must agree with the lexical path: never infer a safe same-vault
/// transfer through a junction, symlink or a nested ambiguous registration.
/// This is a gate admission check, NOT pinned-directory OS authorization.
fn matching_vaults(roots: &[PathBuf], candidate: &Path) -> Option<Vec<String>> {
    // A source must resolve as an existing object. The destination may be new,
    // but its immediate parent must already exist and resolve successfully.
    // Never substitute a raw path when OS identity resolution fails.
    let resolved = match std::fs::symlink_metadata(candidate) {
        Ok(_) => std::fs::canonicalize(candidate).ok()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = std::fs::canonicalize(candidate.parent()?).ok()?;
            parent.join(candidate.file_name()?)
        }
        Err(_) => return None,
    };
    let resolved = canonical_path(&resolved);
    let owners = |path: &Path| {
        let mut matches: Vec<String> = roots
            .iter()
            .filter(|root| strip_root(root, path).is_some())
            .map(|root| identity(root))
            .collect();
        matches.sort();
        matches.dedup();
        matches
    };
    let lexical = owners(candidate);
    let physical = owners(&resolved);
    // macOS /var -> /private/var may be outside the canonical root lexically.
    // Its resolved owner still gives a stable vault classification. But an
    // explicitly matched lexical vault may NEVER resolve to another vault or
    // unregistered storage (symlink/junction escape).
    if !lexical.is_empty() && lexical != physical {
        return None;
    }
    Some(physical)
}

/// PR 04g-3b-7b-0: do not move the PRIMARY file until durable recovery
/// admission is available at BOTH source and destination workspaces.
/// A source-only history marker cannot protect a destination-only Core.
/// Two marker creations in different vaults are NOT one atomic commit.
/// This must be called under the ordinary File Gate, BEFORE 'run()'.
fn admit_vault_relocation(core: &Core, from: &Path, to: &Path) -> Result<(), FileCommandError> {
    let roots = core.ui_state.workspace_roots().map_err(|error| FileCommandError::Task {
        message: format!(
            "Workspace registry cannot be read; refusing to move a primary file before validating history ownership: {error:?}"
        ),
    })?;
    if let (Some(source), Some(destination)) = (
        matching_vaults(&roots, from),
        matching_vaults(&roots, to),
    ) {
        if (source.is_empty() && destination.is_empty())
            || (source.len() == 1 && destination.len() == 1 && source == destination)
        {
            return Ok(());
        }
    }
    Err(FileCommandError::Task {
        message: format!(
            "Cross-workspace rename/move is disabled before changing files: history recovery              has only a source-vault journal, not a verified bilateral admission protocol              (source: {}; destination: {}). Files and version history were not changed.",
            from.display(),
            to.display(),
        ),
    })
}

fn relocating<T>(
    core: &Core,
    from: &Path,
    to: &Path,
    run: impl FnOnce() -> Result<T, FileCommandError>,
) -> Result<(T, Relocation), FileCommandError> {
    let (done, relocation) = {
        // Lock preflight enumeration, rename and all link rewrites as one
        // in-process filesystem critical section. History and UI callbacks
        // run only after releasing the lock.
        let _guard = core.file_mutation_guard()?;
        // Deny before the primary file rename, including directory moves.
        // Never acquire the HistoryService mutex while File Gate is held.
        admit_vault_relocation(core, from, to)?;
        let notes = notes_under(from);
        let done = run()?;
        (done, relocated(&notes, from, to))
    };
    for step in &relocation.moves {
        let from = Path::new(&step.from);
        let to = Path::new(&step.to);
        if let Err(error) = core.history.try_moved(&|| core.known_roots(), from, to) {
            report_history_move_failure(core, from, to, &error);
        }
    }
    Ok((done, relocation))
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

    #[cfg(target_os = "macos")]
    #[test]
    fn history_cross_vault_macos_temp_directory_alias_resolves_to_registered_owner() {
        let directory = tempfile::tempdir().unwrap();
        let raw_root = directory.path().join("vault");
        fs::create_dir_all(&raw_root).unwrap();
        let registered = fs::canonicalize(&raw_root).unwrap();
        let new_note = raw_root.join("note.md");
        assert_eq!(
            matching_vaults(&[registered.clone()], &new_note),
            Some(vec![identity(&registered)])
        );
    }

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
