//! Narrow admission for filenames inside overlapping registered workspaces.
//! A local file rename is not a workspace transfer, but it must not strand
//! history belonging to a second registered root.
use crate::app_core::Core;
use crate::files::error::FileCommandError;
use crate::history::owner;
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

fn local_overlap_owner(
    roots: &[PathBuf],
    owners: &[String],
    from: &Path,
    to: &Path,
) -> Option<usize> {
    if owners.len() < 2 || from.parent() != to.parent() {
        return None;
    }
    let parent = from.parent()?;
    let physical_parent = fs::canonicalize(parent).ok()?;
    if identity(parent) != identity(&canonical_path(&physical_parent)) {
        return None;
    }
    let meta = fs::symlink_metadata(from).ok()?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return None;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x0000_0400 != 0 {
            return None;
        }
    }
    // No overwrite: an existing destination file, alias or unknown I/O
    // failure is not a legitimate same-folder rename.
    if !matches!(fs::symlink_metadata(to), Err(e) if e.kind() == ErrorKind::NotFound) {
        return None;
    }
    let selected = owner::unique_current(roots, from).ok().flatten();
    let fallback = roots.iter().position(|root|
        owners.contains(&identity(root)) && strip_root(root, from).is_some()
    )?;
    // An old per-note Quantum directory cannot be moved by the current
    // HistoryService. An unrelated Quantum archive is not this note's owner.
    for root in roots.iter().filter(|root| owners.contains(&identity(root))) {
        if owner::legacy(root, from).ok()? || owner::legacy(root, to).ok()?
            || owner::current(root, to).ok()?
        {
            return None;
        }
    }
    Some(selected.unwrap_or(fallback))
}

/// Called under File Gate before any primary file or history mutation.
/// Returns a stable registry snapshot for the subsequent history move.
pub(super) fn admit_vault_relocation(
    core: &Core,
    from: &Path,
    to: &Path,
) -> Result<Vec<PathBuf>, FileCommandError> {
    let mut roots = core.ui_state.workspace_roots().map_err(|error| FileCommandError::Task {
        message: format!(
            "Workspace registry cannot be read; refusing to move a primary file before validating history ownership: {error:?}"
        ),
    })?;
    let source = matching_vaults(&roots, from);
    let destination = matching_vaults(&roots, to);
    if let (Some(source), Some(destination)) = (&source, &destination) {
        if (source.is_empty() && destination.is_empty())
            || (source.len() == 1 && destination.len() == 1 && source == destination)
        {
            return Ok(roots);
        }
        if source == destination {
            if let Some(index) = local_overlap_owner(&roots, source, from, to) {
                // HistoryService moves only one root's per-note version tree.
                // Put its actual owner first, regardless of last-opened order.
                // All other registered roots must have NO history for this note.
                let preferred = roots.remove(index);
                roots.insert(0, preferred);
                return Ok(roots);
            }
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
        // Mirror native Workspace::normalize: HistoryService requires the
        // same canonical spelling on macOS (/var) and Windows (\\?\ paths).
        let outer = canonical_path(&outer);
        let inner = outer.join("Английский");
        let data = temp.path().join("appdata");
        fs::create_dir_all(&data).unwrap();
        let core = Core::open(&data, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(outer.to_str().unwrap(), 1).unwrap();
        (core, outer, inner)
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_temp_directory_alias_resolves_to_registered_owner() {
        let directory = tempfile::tempdir().unwrap();
        let raw_root = directory.path().join("vault");
        fs::create_dir_all(&raw_root).unwrap();
        let registered = fs::canonicalize(&raw_root).unwrap();
        let note = raw_root.join("note.md");
        assert_eq!(
            matching_vaults(&[registered.clone()], &note),
            Some(vec![identity(&registered)])
        );
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
        assert_eq!(core.history.page(&|| core.known_roots(), &old, 0, 8).total, 0);
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
