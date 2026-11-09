//! Non-destructive, opt-in copying of an *inactive* knowledge base.
//! Never remove source or partially copied folders. Failed copies are not registered.
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use masonry::app::RenderRoot;

use super::{App, Popup};
use crate::i18n::{t, t_with};

const MAX_ENTRIES: usize = 200_000;
const MAX_BYTES: u64 = 50 * 1024 * 1024 * 1024;
const MAX_NAME_ATTEMPTS: usize = 1_000;

#[derive(Default)]
struct Budget {
    entries: usize,
    bytes: u64,
}

fn copy_contents(source: &Path, destination: &Path, budget: &mut Budget) -> io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let original = entry.path();
        let target = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&original)?;
        // Do not follow Unix symlinks or Windows directory junctions.
        if metadata.file_type().is_symlink() {
            return Err(io::Error::new(ErrorKind::InvalidData, "linked files and folders are not copied"));
        }
        budget.entries = budget.entries.checked_add(1).ok_or_else(|| io::Error::other("too many entries"))?;
        if budget.entries > MAX_ENTRIES {
            return Err(io::Error::other("knowledge base exceeds copy entry limit"));
        }
        if metadata.is_dir() {
            fs::create_dir(&target)?;
            copy_contents(&original, &target, budget)?;
        } else if metadata.is_file() {
            budget.bytes = budget.bytes.checked_add(metadata.len()).ok_or_else(|| io::Error::other("copy size overflow"))?;
            if budget.bytes > MAX_BYTES {
                return Err(io::Error::other("knowledge base exceeds copy size limit"));
            }
            let mut from = File::open(&original)?;
            // Never overwrite anything, even if another process creates a collision.
            let mut to = OpenOptions::new().write(true).create_new(true).open(&target)?;
            io::copy(&mut from, &mut to)?;
            to.sync_all()?;
        } else {
            return Err(io::Error::new(ErrorKind::InvalidData, "unsupported special file"));
        }
    }
    Ok(())
}

fn duplicate_folder(source: &Path, suffix: &str) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(source).map_err(|err| err.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("source must be a real directory".into());
    }
    let parent = source.parent().ok_or("source has no parent folder")?;
    let name = source.file_name().ok_or("source has no folder name")?.to_string_lossy();
    if name.is_empty() || suffix.trim().is_empty() {
        return Err("invalid knowledge-base name".into());
    }
    for attempt in 1..=MAX_NAME_ATTEMPTS {
        let label = if attempt == 1 { format!("{name} ({suffix})") } else { format!("{name} ({suffix} {attempt})") };
        let destination = parent.join(label);
        match fs::create_dir(&destination) {
            Ok(()) => {
                let mut budget = Budget::default();
                if let Err(error) = copy_contents(source, &destination, &mut budget) {
                    // Removing an incomplete copy would be a destructive directory operation.
                    // Keep it unregistered and tell the user where to inspect it.
                    return Err(format!("{error}. Incomplete copy retained at {}", destination.display()));
                }
                return Ok(destination);
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("no unused copy folder name available".into())
}

impl App {
    pub(in crate::app) fn start_duplicate_workspace(&self, source: PathBuf) {
        let suffix = t("workspaces.copySuffix");
        let sender = self.picked_tx.clone();
        let wake = Arc::clone(&self.workspace.wake);
        std::thread::spawn(move || {
            let result = duplicate_folder(&source, &suffix);
            if sender.send(super::super::book::Picked::WorkspaceDuplicated(result)).is_ok() {
                wake();
            }
        });
    }

    pub(in crate::app) fn on_workspace_duplicated(&mut self, root: &mut RenderRoot, result: Result<PathBuf, String>) -> bool {
        let error = match result {
            Ok(path) => self.workspace.core.ui_state
                .resolve_workspace(&path.to_string_lossy(), crate::dates::now_ms())
                .err()
                .map(|err| format!("{err:?}")),
            Err(reason) => Some(reason),
        };
        if let Some(reason) = &error {
            report!("база не продублирована: {reason}");
        }
        if matches!(self.popup, Some(Popup::Workspaces(_))) {
            self.close_workspaces(root);
            self.show_workspaces(root, error.map(|reason| t_with("workspaces.duplicateFailed", &[("reason", &reason)])));
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_preserves_source_and_copies_nested_binary_files() {
        let temp = crate::test_dir::tempdir();
        let source = temp.path().join("Notes");
        fs::create_dir(&source).unwrap();
        fs::create_dir(source.join("media")).unwrap();
        fs::write(source.join("a.md"), "original").unwrap();
        fs::write(source.join("media").join("a.bin"), [0_u8, 255, 17]).unwrap();
        let first = duplicate_folder(&source, "copy").unwrap();
        let second = duplicate_folder(&source, "copy").unwrap();
        assert_eq!(first.file_name().unwrap(), "Notes (copy)");
        assert_eq!(second.file_name().unwrap(), "Notes (copy 2)");
        assert_eq!(fs::read(first.join("media").join("a.bin")).unwrap(), [0, 255, 17]);
        assert_eq!(fs::read_to_string(second.join("a.md")).unwrap(), "original");
        fs::write(first.join("a.md"), "changed copy").unwrap();
        assert_eq!(fs::read_to_string(source.join("a.md")).unwrap(), "original");
    }

    #[test]
    fn missing_source_fails_without_creating_any_folder() {
        let temp = crate::test_dir::tempdir();
        assert!(duplicate_folder(&temp.path().join("missing"), "copy").is_err());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_in_source_is_rejected_without_touching_target_data() {
        use std::os::unix::fs::symlink;
        let temp = crate::test_dir::tempdir();
        let source = temp.path().join("Source");
        fs::create_dir(&source).unwrap();
        fs::write(temp.path().join("outside.md"), "UNCHANGED").unwrap();
        symlink(temp.path().join("outside.md"), source.join("link.md")).unwrap();
        let error = duplicate_folder(&source, "copy").unwrap_err();
        assert!(error.contains("Incomplete copy retained"), "{error}");
        assert_eq!(fs::read_to_string(temp.path().join("outside.md")).unwrap(), "UNCHANGED");
    }
}
