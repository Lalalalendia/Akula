//! Trusted host-side resolution of a saved Language source's document ID.
//! An active DB path is only identity metadata. Never treat this as a file
//! access grant, a content snapshot, or permission to bypass File Gate.

use std::path::{Component, Path, PathBuf};
use uuid::Uuid;
use aquilum_core::search::paths::is_markdown;

use super::Session;
use aquilum_core::Core;

impl Session { pub(crate) fn study_root(&self) -> &Path { &self.root }
    /// Find the current, existing Markdown path belonging to a document ID
    /// in THIS workspace. No file contents are read. The caller must still
    /// obtain a fresh authorized read and revision from the trusted host.
    pub(crate) fn study_document_path(&self, core: &Core, document_id: Uuid) -> Option<PathBuf> {
        let relative = core.ui_state.active_document_relative(self.workspace_id, document_id)
            .ok()??;
        let root = std::fs::canonicalize(&self.root).ok()?;
        let mut candidate = root.clone();
        for component in Path::new(&relative).components() {
            let Component::Normal(name) = component else { return None };
            candidate.push(name);
            // Explicitly reject symlink aliases, including parent directories.
            // A saved source ID must not silently change ownership through an alias.
            if std::fs::symlink_metadata(&candidate).ok()?.file_type().is_symlink() {
                return None;
            }
        }
        let canonical = std::fs::canonicalize(candidate).ok()?;
        (canonical.starts_with(&root) && canonical.is_file() && is_markdown(&canonical))
            .then_some(canonical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn stored_note_identity_resolves_only_inside_its_active_workspace() {
        let vault = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::create_dir(vault.path().join("Lessons")).unwrap();
        let path = vault.path().join("Lessons").join("a.md");
        std::fs::write(&path, "| Q | A |\n|---|---|\n| eat | есть |").unwrap();
        let core = Core::open(data.path(), Arc::new(|_| {}));
        let (session, _) = Session::open(&core, vault.path()).unwrap();
        let doc = session.document(&core, &path).unwrap();
        assert_eq!(session.study_document_path(&core, doc),
            Some(std::fs::canonicalize(&path).unwrap()));
        assert_eq!(session.study_document_path(&core, Uuid::new_v4()), None);

        let foreign = tempfile::tempdir().unwrap();
        let (other_session, _) = Session::open(&core, foreign.path()).unwrap();
        assert_eq!(other_session.study_document_path(&core, doc), None);
        core.ui_state.mark_document_missing(doc, 100).unwrap();
        assert_eq!(session.study_document_path(&core, doc), None);
    }

    #[test]
    fn renamed_document_is_resolved_by_identity_not_stale_saved_path() {
        let vault = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let before = vault.path().join("before.md");
        let after = vault.path().join("after.md");
        std::fs::write(&before, "note").unwrap();
        let core = Core::open(data.path(), Arc::new(|_| {}));
        let (session, _) = Session::open(&core, vault.path()).unwrap();
        let doc = session.document(&core, &before).unwrap();
        std::fs::rename(&before, &after).unwrap();
        core.ui_state.rename_document(doc, "after.md").unwrap();
        assert_eq!(session.study_document_path(&core, doc),
            Some(std::fs::canonicalize(&after).unwrap()));
        let forged = core.ui_state.resolve_document(session.workspace_id, "../escape.md").unwrap();
        assert_eq!(session.study_document_path(&core, forged), None);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_note_or_directory_does_not_rebind_saved_identity() {
        use std::os::unix::fs::symlink;
        let vault = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let secret = other.path().join("private.md");
        std::fs::write(&secret, "secret").unwrap();
        symlink(&secret, vault.path().join("alias.md")).unwrap();
        symlink(other.path(), vault.path().join("linked")).unwrap();

        let core = Core::open(data.path(), Arc::new(|_| {}));
        let (session, _) = Session::open(&core, vault.path()).unwrap();
        let direct = session.document(&core, &vault.path().join("alias.md")).unwrap();
        let nested = session.document(&core, &vault.path().join("linked/private.md")).unwrap();
        assert_eq!(session.study_document_path(&core, direct), None);
        assert_eq!(session.study_document_path(&core, nested), None);
    }
}
