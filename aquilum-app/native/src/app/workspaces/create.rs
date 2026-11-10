use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use masonry::app::RenderRoot;
use super::{App, Outcome};
use crate::i18n::{t, t_with};

/// Creates only a new directory; never replaces or deletes existing data.
fn create_named(parent: &Path, name: &str) -> Result<PathBuf, String> {
    if !parent.is_dir() { return Err("parent directory is unavailable".into()); }
    let safe = crate::file_ops::sanitize_file_name(name);
    if safe.is_empty() { return Err("new knowledge-base name is empty".into()); }
    for attempt in 0..1_000 {
        let label = if attempt == 0 { safe.clone() } else { format!("{safe} ({attempt})") };
        let dest = parent.join(label);
        match std::fs::create_dir(&dest) {
            Ok(()) => return Ok(dest),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err.to_string()),
        }
    }
    Err("no unused knowledge-base name found".into())
}

impl App {
    pub(in crate::app) fn pick_new_workspace(&self) -> Outcome {
        let initial = self.tree.root().and_then(Path::parent).map(Path::to_path_buf);
        let name = t("workspaces.newName");
        let title = t("workspaces.pickParent");
        let sender = self.picked_tx.clone();
        let wake = Arc::clone(&self.workspace.wake);
        std::thread::spawn(move || {
            let mut picker = rfd::FileDialog::new().set_title(title.as_str());
            if let Some(initial) = initial { picker = picker.set_directory(initial); }
            let Some(parent) = picker.pick_folder() else { return };
            let created = create_named(&parent, &name);
            if sender.send(super::super::book::Picked::WorkspaceCreated(created)).is_ok() {
                wake();
            }
        });
        Outcome::default()
    }

    pub(in crate::app) fn on_workspace_created(&mut self, root: &mut RenderRoot, result: Result<PathBuf, String>) -> bool {
        match result {
            Ok(path) => self.switch_workspace(root, path).title,
            Err(reason) => {
                self.close_workspaces(root);
                self.show_workspaces(root, Some(t_with("workspaces.createFailed", &[("reason", &reason)])));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_creation_never_overwrites_existing_files() {
        let temp = crate::test_dir::tempdir();
        let existing = temp.path().join("New knowledge base");
        std::fs::create_dir(&existing).unwrap();
        std::fs::write(existing.join("existing.md"), "KEEP").unwrap();
        let first = create_named(temp.path(), "New knowledge base").unwrap();
        let second = create_named(temp.path(), "New knowledge base").unwrap();
        assert_ne!(first, existing);
        assert_ne!(first, second);
        assert!(first.is_dir() && second.is_dir());
        assert_eq!(std::fs::read_to_string(existing.join("existing.md")).unwrap(), "KEEP");
    }

    #[test]
    fn invalid_parent_and_name_fail_closed() {
        let temp = crate::test_dir::tempdir();
        let missing = temp.path().join("missing");
        assert!(create_named(&missing, "Test").is_err());
        assert!(!missing.exists());
        assert!(create_named(temp.path(), "/?").is_err());
    }
}
