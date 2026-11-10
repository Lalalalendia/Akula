//! Duplicate only inactive knowledge bases; never delete the source or partial copies.
mod snapshot;

use std::path::PathBuf;
use std::sync::Arc;

#[cfg(test)]
use std::fs;
use masonry::app::RenderRoot;

use super::super::{App, Popup};
use crate::i18n::{t, t_with};
use snapshot::duplicate_folder;

impl App {
    pub(in crate::app) fn start_duplicate_workspace(&self, source: PathBuf) {
        let suffix = t("workspaces.copySuffix");
        let sender = self.picked_tx.clone();
        let wake = Arc::clone(&self.workspace.wake);
        std::thread::spawn(move || {
            let result = duplicate_folder(&source, &suffix);
            if sender.send(super::super::super::book::Picked::WorkspaceDuplicated(result)).is_ok() {
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
        assert!(error.contains("linked"), "{error}");
        assert!(!temp.path().join("Source (copy)").exists());
        assert_eq!(fs::read_to_string(temp.path().join("outside.md")).unwrap(), "UNCHANGED");
    }
}
