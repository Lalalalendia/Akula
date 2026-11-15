//! Select a note's persisted history, not the last workspace that was opened.
//! This is a pathname preflight only; File Gate still owns mutation admission.
use super::store;
use crate::search::paths::{identity, strip_root};
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Component, Path, PathBuf};

fn denied(message: &str) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, message)
}

fn ordinary_directory(path: &Path) -> io::Result<bool> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x0000_0400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if !meta.is_dir() || meta.file_type().is_symlink() || reparse {
        return Err(denied("history path has an alias or non-directory entry"));
    }
    Ok(true)
}

fn relative_note<'a>(root: &Path, note: &'a Path) -> io::Result<&'a Path> {
    let relative = strip_root(root, note).filter(|r| !r.as_os_str().is_empty())
        .ok_or_else(|| denied("note is outside selected history root"))?;
    if !relative.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(denied("invalid note history path component"));
    }
    Ok(relative)
}

pub(crate) fn current(root: &Path, note: &Path) -> io::Result<bool> {
    let relative = relative_note(root, note)?;
    let target = store::history_dir(root, relative);
    store::checked_history_note_directory(root, &target)?;
    ordinary_directory(&target)
}

/// The presence of .quantum/history for an UNRELATED note is not a reason
/// to block renaming this note. Refuse an actual legacy version directory,
/// unsafe ancestor, junction or unreadable component.
pub(crate) fn legacy(root: &Path, note: &Path) -> io::Result<bool> {
    let relative = relative_note(root, note)?;
    let mut current = root.to_path_buf();
    if !ordinary_directory(&current)? {
        return Err(denied("workspace root disappeared before legacy history inspection"));
    }
    let path = Path::new(".quantum").join("history").join(relative);
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err(denied("invalid legacy history component"));
        };
        current.push(name);
        if !ordinary_directory(&current)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Returns a unique registered root with a per-note history directory.
/// More than one independent history, or an uninspectable root, is an error,
/// not authority to choose whichever workspace was opened most recently.
pub(crate) fn unique_current(roots: &[PathBuf], note: &Path) -> io::Result<Option<usize>> {
    let mut matched: Option<usize> = None;
    for (index, root) in roots.iter().enumerate() {
        if strip_root(root, note).is_none() {
            continue;
        }
        if current(root, note)? {
            if let Some(previous) = matched {
                if identity(roots.get(previous).expect("previous index came from roots").as_path()) != identity(root.as_path()) {
                    return Err(denied("more than one registered workspace stores this note's history"));
                }
            } else {
                matched = Some(index);
            }
        }
    }
    Ok(matched)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrelated_legacy_note_does_not_own_another_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Книги");
        let nested = root.join("Английский");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(root.join(".quantum/history/Другая.md")).unwrap();
        assert!(!legacy(&root, &nested.join("Без названия.md")).unwrap());
        assert!(legacy(&root, &root.join("Другая.md")).unwrap());
    }

    #[test]
    fn duplicate_current_history_refuses_owner_selection() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Книги");
        let nested = root.join("Английский");
        fs::create_dir_all(&nested).unwrap();
        let note = nested.join("Без названия.md");
        fs::create_dir_all(store::history_dir(&root, Path::new("Английский/Без названия.md"))).unwrap();
        fs::create_dir_all(store::history_dir(&nested, Path::new("Без названия.md"))).unwrap();
        assert!(unique_current(&[nested, root], &note).is_err());
    }

    #[test]
    fn duplicate_registration_of_one_physical_root_is_not_independent_history() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Книги");
        fs::create_dir_all(&root).unwrap();
        let note = root.join("Без названия.md");
        fs::create_dir_all(store::history_dir(&root, Path::new("Без названия.md"))).unwrap();
        assert_eq!(unique_current(&[root.clone(), root], &note).unwrap(), Some(0));
    }

    #[cfg(unix)]
    #[test]
    fn legacy_symlink_is_not_absent_history() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Книги");
        fs::create_dir_all(&root).unwrap();
        symlink(dir.path(), root.join(".quantum")).unwrap();
        assert!(legacy(&root, &root.join("Без названия.md")).is_err());
    }
}
