//! Narrow admission for filenames inside overlapping registered workspaces.
//! A local file rename is not a workspace transfer, but it must not strand
//! history belonging to a second registered root.
use crate::app_core::Core;
use crate::files::error::FileCommandError;
use crate::history::store;
use crate::search::paths::{canonical_path, identity, strip_root};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

fn matching_vaults(roots: &[PathBuf], candidate: &Path) -> Option<Vec<String>> {
    let resolved = match fs::symlink_metadata(candidate) {
        Ok(_) => fs::canonicalize(candidate).ok()?,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let parent = fs::canonicalize(candidate.parent()?).ok()?;
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
    // Do not admit a symlink/junction escape via physical ownership alone.
    if !lexical.is_empty() && lexical != physical {
        return None;
    }
    Some(physical)
}

fn absent_without_alias(root: &Path, note: &Path) -> bool {
    let Some(relative) = strip_root(root, note) else {
        return false;
    };
    let directory = store::history_dir(root, relative);
    if store::checked_history_note_directory(root, &directory).is_err() {
        return false;
    }
    matches!(
        fs::symlink_metadata(directory),
        Err(error) if error.kind() == ErrorKind::NotFound
    )
}

fn no_legacy_history(root: &Path) -> bool {
    let legacy = root.join(".quantum").join("history");
    matches!(
        fs::symlink_metadata(legacy),
        Err(error) if error.kind() == ErrorKind::NotFound
    )
}

fn local_overlap_is_safe(
    roots: &[PathBuf],
    owners: &[String],
    from: &Path,
    to: &Path,
) -> bool {
    if owners.len() < 2 || from.parent() != to.parent() {
        return false;
    }
    let Some(parent) = from.parent() else {
        return false;
    };
    let Ok(physical_parent) = fs::canonicalize(parent) else {
        return false;
    };
    // A reparse/symlink parent is not evidence of a local physical rename.
    if identity(parent) != identity(&canonical_path(&physical_parent)) {
        return false;
    }
    let Ok(metadata) = fs::symlink_metadata(from) else {
        return false;
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x0000_0400 != 0 {
            return false;
        }
    }
    // Occupied destinations and Windows case-only aliases retain the
    // conservative original gate; no accidental overwrite is admitted here.
    if !matches!(fs::symlink_metadata(to), Err(error) if error.kind() == ErrorKind::NotFound) {
        return false;
    }
    let mut matches = roots
        .iter()
        .filter(|root| owners.contains(&identity(root)) && strip_root(root, from).is_some());
    let Some(selected) = matches.next() else {
        return false;
    };
    // HistoryService selects the first registered matching root. Any other
    // root with old/new (or legacy) history must stay on HOLD: relocating
    // only the first history would orphan versions from the other workspace.
    matches.all(|root| {
        identity(root) == identity(selected)
            || (no_legacy_history(root)
                && absent_without_alias(root, from)
                && absent_without_alias(root, to))
    })
}

/// Called under File Gate before any primary file or history mutation.
/// Returns a stable registry snapshot for the subsequent history move.
pub(super) fn admit_vault_relocation(
    core: &Core,
    from: &Path,
    to: &Path,
) -> Result<Vec<PathBuf>, FileCommandError> {
    let roots = core.ui_state.workspace_roots().map_err(|error| FileCommandError::Task {
        message: format!(
            "Workspace registry cannot be read; refusing to move a primary file before validating history ownership: {error:?}"
        ),
    })?;
    let source = matching_vaults(&roots, from);
    let destination = matching_vaults(&roots, to);
    if let (Some(source), Some(destination)) = (&source, &destination) {
        if (source.is_empty() && destination.is_empty())
            || (source.len() == 1 && destination.len() == 1 && source == destination)
            || (source == destination && local_overlap_is_safe(&roots, source, from, to))
        {
            return Ok(roots);
        }
    }
    let detail = match (&source, &destination) {
        (Some(a), Some(b)) if a.len() > 1 && a == b => {
            "overlapping registered workspace roots or independent history prevent a safe local rename; remove the redundant workspace registration (not its files) or resolve its history ownership"
        }
        (Some(a), Some(b)) if a != b => "source and destination belong to different registered workspace roots",
        _ => "path ownership is unknown or an alias does not match its physical workspace",
    };
    Err(FileCommandError::Task {
        message: format!(
            "Cross-workspace rename/move is disabled before changing files: {detail}; history recovery has only a source-vault journal, not a verified bilateral admission protocol (source: {}; destination: {}). Files and version history were not changed.",
            from.display(),
            to.display(),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::CoreEvent;
    use crate::history::Source;
    use std::sync::Arc;

    fn core_with_outer(temp: &tempfile::TempDir) -> (Arc<Core>, PathBuf, PathBuf) {
        let outer = temp.path().join("Книги");
        let inner = outer.join("Английский");
        fs::create_dir_all(&inner).unwrap();
        let data = temp.path().join("appdata");
        fs::create_dir_all(&data).unwrap();
        let core = Core::open(&data, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(outer.to_str().unwrap(), 1).unwrap();
        (core, outer, inner)
    }

    #[test]
    fn same_directory_rename_in_nested_registered_workspace_keeps_history() {
        let temp = tempfile::tempdir().unwrap();
        let (core, _outer, inner) = core_with_outer(&temp);
        core.ui_state.resolve_workspace(inner.to_str().unwrap(), 2).unwrap();
        let old = inner.join("Без названия.md");
        let new = inner.join("Грамматика английского языка.md");
        crate::files::gate::create(&core, &old, "текст", Source::Me, None).unwrap();
        assert!(core.history.page(&|| core.known_roots(), &old, 0, 8).total > 0);
        crate::files::gate::rename(&core, &old, &new).unwrap();
        assert!(!old.exists());
        assert_eq!(fs::read_to_string(&new).unwrap(), "текст");
        assert!(core.history.page(&|| core.known_roots(), &new, 0, 8).total > 0);
    }

    #[test]
    fn second_workspace_existing_history_refuses_rename_without_touching_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let (core, outer, inner) = core_with_outer(&temp);
        let old = inner.join("Без названия.md");
        let new = inner.join("Грамматика английского языка.md");
        crate::files::gate::create(&core, &old, "не терять", Source::Me, None).unwrap();
        assert!(store::history_dir(&outer, old.strip_prefix(&outer).unwrap()).exists());
        core.ui_state.resolve_workspace(inner.to_str().unwrap(), 2).unwrap();
        assert!(crate::files::gate::rename(&core, &old, &new).is_err());
        assert_eq!(fs::read_to_string(&old).unwrap(), "не терять");
        assert!(!new.exists());
    }

    #[test]
    fn transfer_out_of_inner_root_stays_denied() {
        let temp = tempfile::tempdir().unwrap();
        let (core, outer, inner) = core_with_outer(&temp);
        core.ui_state.resolve_workspace(inner.to_str().unwrap(), 2).unwrap();
        let old = inner.join("Note.md");
        let new = outer.join("Note.md");
        fs::write(&old, "keep").unwrap();
        assert!(crate::files::gate::rename(&core, &old, &new).is_err());
        assert_eq!(fs::read_to_string(&old).unwrap(), "keep");
        assert!(!new.exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_source_is_not_admitted_by_overlap_exception() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let (core, _outer, inner) = core_with_outer(&temp);
        core.ui_state.resolve_workspace(inner.to_str().unwrap(), 2).unwrap();
        let target = inner.join("Real.md");
        let source = inner.join("Alias.md");
        let renamed = inner.join("Changed.md");
        fs::write(&target, "keep").unwrap();
        symlink(&target, &source).unwrap();
        assert!(crate::files::gate::rename(&core, &source, &renamed).is_err());
        assert!(source.is_symlink());
        assert!(!renamed.exists());
    }
}
