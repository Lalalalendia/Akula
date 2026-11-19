//! LANG-03m: explicit, local-only backup commands for the active note.
//!
//! A file picker does not grant access to the vault or other notes. Re-admit
//! the clean, registered editor after the picker; never copy source Markdown,
//! overwrite an existing destination, or auto-attach imported coordinates.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

use aquilum_core::text::annotation_backup::{export_document, import_document, BackupError};
use aquilum_core::text::annotation_store::AnnotationStore;
use masonry::app::RenderRoot;

use super::super::App;
use super::annotation_ui::AuthorizedEditor;
use crate::i18n::t;

const MAX_BACKUP_BYTES: u64 = 4 * 1024 * 1024;

fn same_document(before: &AuthorizedEditor, now: &AuthorizedEditor) -> bool {
    before.path == now.path && before.document == now.document
        && before.baseline_hash == now.baseline_hash && before.body == now.body
}

/// A clean saver ACK alone is insufficient if the current editor still has
/// an unsaved full-note change; recheck the exact complete Markdown digest.
fn clean_source(app: &App, root: &RenderRoot) -> Option<AuthorizedEditor> {
    let source = app.authorized_annotation_editor(root)?;
    let full_hash = aquilum_core::files::document::hash_bytes(
        app.text.to_string().as_bytes(),
    );
    (full_hash == source.baseline_hash).then_some(source)
}

/// Export never creates the sidecar on a cold vault or follows a DB symlink.
fn existing_sidecar(path: &Path) -> bool {
    fs::symlink_metadata(path).ok()
        .is_some_and(|m| m.file_type().is_file())
}

fn may_open_sidecar(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(m) => m.file_type().is_file(),
        Err(e) => e.kind() == io::ErrorKind::NotFound,
    }
}

fn read_backup(path: &Path) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a regular file"));
    }
    if metadata.len() > MAX_BACKUP_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "backup too large"));
    }
    let mut bytes = Vec::new();
    File::open(path)?.take(MAX_BACKUP_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BACKUP_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "backup too large"));
    }
    Ok(bytes)
}

fn create_backup(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() as u64 > MAX_BACKUP_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "backup too large"));
    }
    // create_new is deliberately used instead of File::create, because a file
    // picker is not consent to replace any existing backup or Markdown file.
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn import_notice(error: BackupError) -> &'static str {
    match error {
        BackupError::AlreadyPresent => "editor.annotation.backupExisting",
        BackupError::ForeignDocument => "editor.annotation.backupForeign",
        BackupError::TooLarge => "editor.annotation.backupTooLarge",
        _ => "editor.annotation.backupInvalid",
    }
}

impl App {
    pub(in crate::app) fn export_annotation_backup(&mut self, root: &mut RenderRoot) {
        let Some(before) = clean_source(self, root) else {
            self.study_notice(root, &t("editor.annotation.unavailable"));
            return;
        };
        let db = self.workspace.data_dir.join("text-annotations.sqlite");
        if !existing_sidecar(&db) {
            self.study_notice(root, &t("editor.annotation.backupNoData"));
            return;
        }
        let filename = self.note.as_ref()
            .and_then(|note| note.path.file_stem())
            .and_then(|name| name.to_str()).unwrap_or("note");
        let Some(destination) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .set_file_name(format!("{filename}.annotations.json"))
            .save_file() else { return };
        if !clean_source(self, root).is_some_and(|now| same_document(&before, &now)) {
            self.study_notice(root, &t("editor.annotation.changed"));
            return;
        }
        let payload = AnnotationStore::open(&db).and_then(|store| store.load())
            .ok().and_then(|snapshot| {
                snapshot.records.iter().any(|r| r.anchor.document == before.document)
                    .then(|| export_document(&snapshot, &before.document).ok()).flatten()
            });
        let Some(bytes) = payload else {
            self.study_notice(root, &t("editor.annotation.backupNoData"));
            return;
        };
        if create_backup(&destination, &bytes).is_ok() {
            self.study_notice(root, &t("editor.annotation.backupSaved"));
        } else {
            self.study_notice(root, &t("editor.annotation.backupWriteFailed"));
        }
    }

    pub(in crate::app) fn import_annotation_backup(&mut self, root: &mut RenderRoot) {
        let Some(before) = clean_source(self, root) else {
            self.study_notice(root, &t("editor.annotation.unavailable"));
            return;
        };
        let Some(source) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"]).pick_file() else { return };
        if !clean_source(self, root).is_some_and(|now| same_document(&before, &now)) {
            self.study_notice(root, &t("editor.annotation.changed"));
            return;
        }
        let Ok(bytes) = read_backup(&source) else {
            self.study_notice(root, &t("editor.annotation.backupInvalid"));
            return;
        };
        if !clean_source(self, root).is_some_and(|now| same_document(&before, &now)) {
            self.study_notice(root, &t("editor.annotation.changed"));
            return;
        }
        let db = self.workspace.data_dir.join("text-annotations.sqlite");
        if !may_open_sidecar(&db) {
            self.study_notice(root, &t("editor.annotation.backupInvalid"));
            return;
        }
        let result = AnnotationStore::open(&db).map_err(BackupError::Store)
            .and_then(|mut store| {
                let snapshot = store.load().map_err(BackupError::Store)?;
                import_document(&mut store, snapshot.revision, &before.document, &bytes)
            });
        match result {
            Ok(_) => {
                // Imported anchors are held, never painted from backup offsets.
                self.hide_annotation_overlay(root);
                self.study_notice(root, &t("editor.annotation.backupImported"));
            }
            Err(error) => self.study_notice(root, &t(import_notice(error))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_creation_does_not_replace_an_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("lesson.annotations.json");
        fs::write(&destination, b"existing").unwrap();
        assert_eq!(create_backup(&destination, b"new").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(destination).unwrap(), b"existing");
    }

    #[test]
    fn backup_reader_rejects_directories_and_oversized_files() {
        let temp = tempfile::tempdir().unwrap();
        assert!(read_backup(temp.path()).is_err());
        let destination = temp.path().join("oversized.json");
        fs::write(&destination, vec![b'x'; MAX_BACKUP_BYTES as usize + 1]).unwrap();
        assert!(read_backup(&destination).is_err());
    }

    #[test]
    fn sidecar_admission_is_non_creating_and_rejects_non_files() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("sidecar.sqlite");
        assert!(!existing_sidecar(&db));
        assert!(may_open_sidecar(&db));
        assert!(!db.exists());
        fs::create_dir(&db).unwrap();
        assert!(!existing_sidecar(&db));
        assert!(!may_open_sidecar(&db));
    }
}
