//! Agent prewrite must not read a replaced foreign directory through a pathname.
use super::{write_retrying, Vault};
use crate::app_core::{Core, CoreEvent};
use crate::history::Source;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::sync::Arc;

fn setup() -> (tempfile::TempDir, Arc<Core>, Vault, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    let app = dir.path().join("app");
    fs::create_dir_all(root.join("Раздел")).unwrap();
    fs::create_dir(&app).unwrap();
    fs::write(root.join("Раздел/Заметка.md"), b"inside\r\ntext").unwrap();
    let vault = Vault {
        root: fs::canonicalize(&root).unwrap(),
        visible: false,
    };
    let note = vault.note("Раздел/Заметка.md").unwrap();
    let core = Core::open(&app, Arc::new(|_: CoreEvent| {}));
    (dir, core, vault, note)
}

#[test]
fn prewrite_preserves_raw_revision_and_normalizes_crlf() {
    let (_dir, core, vault, note) = setup();
    let result = write_retrying(&core, &vault, &note, Source::Agent, None, |current| {
        assert_eq!(current, "inside\ntext");
        Ok(current.to_owned())
    }).unwrap();
    assert_eq!(result, (false, "inside\ntext".to_owned()));
    assert_eq!(fs::read(&note).unwrap(), b"inside\r\ntext");
}

#[test]
fn prewrite_fails_closed_after_parent_is_replaced_by_outside_symlink() {
    let (_dir, core, vault, note) = setup();
    let external = tempfile::tempdir().unwrap();
    fs::write(external.path().join("Заметка.md"), b"external secret").unwrap();
    let old_parent = note.parent().unwrap();
    fs::rename(old_parent, vault.root.join("moved-original")).unwrap();
    symlink(external.path(), old_parent).unwrap();

    let result = write_retrying(&core, &vault, &note, Source::Agent, None, |_| {
        Ok("corrupted".to_owned())
    });
    assert!(result.is_err(), "foreign symlink may not supply prewrite bytes");
    assert_eq!(fs::read(external.path().join("Заметка.md")).unwrap(), b"external secret");
    assert_eq!(fs::read(vault.root.join("moved-original/Заметка.md")).unwrap(),
        b"inside\r\ntext");
}
