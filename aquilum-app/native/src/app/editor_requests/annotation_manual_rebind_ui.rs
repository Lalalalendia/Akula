//! LANG-03p: manual HOLD recovery is an explicit action on selected text.
//!
//! The context menu does not convey a read grant. Before any SQLite write,
//! re-admit the exact registered, clean, saved Markdown document and user
//! selection. A backup's opaque IDs never authorize reading a note.
//! No automatic phrase search, source-text copy, or Markdown write is done.

use std::fs;
use std::path::Path;

use aquilum_core::text::annotation_manual_rebind::{
    bind_explicit_selection, ManualRebindError,
};
use aquilum_core::text::annotation_store::AnnotationStore;
use aquilum_core::text::annotations::capture;
use masonry::app::RenderRoot;

use super::super::App;
use super::annotation_ui::AuthorizedEditor;
use crate::i18n::t;

fn clean_selection(app: &App, root: &RenderRoot) -> Option<AuthorizedEditor> {
    let source = app.authorized_annotation_editor(root)?;
    let hash = aquilum_core::files::document::hash_bytes(
        app.text.to_string().as_bytes(),
    );
    (hash == source.baseline_hash).then_some(source)
}

fn same_selection(before: &AuthorizedEditor, now: &AuthorizedEditor) -> bool {
    before.path == now.path
        && before.document == now.document
        && before.baseline_hash == now.baseline_hash
        && before.body == now.body
        && before.selection == now.selection
}

fn regular_sidecar(path: &Path) -> bool {
    fs::symlink_metadata(path).ok()
        .is_some_and(|meta| meta.file_type().is_file())
}

fn notice(error: ManualRebindError) -> &'static str {
    match error {
        ManualRebindError::NoMatchingHeld => "editor.annotation.rebindNoMatch",
        ManualRebindError::AmbiguousHeld => "editor.annotation.rebindAmbiguous",
        ManualRebindError::RevisionConflict => "editor.annotation.changed",
        _ => "editor.annotation.saveFailed",
    }
}

impl App {
    pub(in crate::app) fn rebind_selected_annotation(&mut self, root: &mut RenderRoot) {
        let Some(before) = clean_selection(self, root) else {
            self.study_notice(root, &t("editor.annotation.unavailable"));
            return;
        };
        if capture(&before.document, "pending-selection", &before.body,
            before.selection.clone()).is_err()
        {
            self.study_notice(root, &t("editor.annotation.selectText"));
            return;
        }

        let db = self.workspace.data_dir.join("text-annotations.sqlite");
        if !regular_sidecar(&db) {
            self.study_notice(root, &t("editor.annotation.rebindNoMatch"));
            return;
        }
        let Ok(mut store) = AnnotationStore::open(&db) else {
            self.study_notice(root, &t("editor.annotation.saveFailed"));
            return;
        };
        let Ok(snapshot) = store.load() else {
            self.study_notice(root, &t("editor.annotation.saveFailed"));
            return;
        };

        // Revalidate after loading the sidecar, immediately before CAS.
        // A closed tab, edited selection, stale file or foreign vault fails.
        if !clean_selection(self, root).is_some_and(|now|
            same_selection(&before, &now))
        {
            self.study_notice(root, &t("editor.annotation.changed"));
            return;
        }
        let result = bind_explicit_selection(
            &mut store, snapshot.revision, &before.document,
            &before.body, before.selection,
        );
        drop(store);
        match result {
            Ok(_) => {
                self.refresh_annotation_overlay(root);
                self.study_notice(root, &t("editor.annotation.rebindSaved"));
            }
            Err(error) => self.study_notice(root, &t(notice(error))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_rejects_a_missing_or_directory_sidecar_without_creating_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("text-annotations.sqlite");
        assert!(!regular_sidecar(&path));
        assert!(!path.exists());
        fs::create_dir(&path).unwrap();
        assert!(!regular_sidecar(&path));
    }

    #[cfg(unix)]
    #[test]
    fn admission_rejects_a_symlinked_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("source.sqlite");
        fs::write(&target, b"fixture").unwrap();
        let link = dir.path().join("text-annotations.sqlite");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(!regular_sidecar(&link));
        assert_eq!(fs::read(target).unwrap(), b"fixture");
    }
}
