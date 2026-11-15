use super::*;
use crate::app_core::CoreEvent;
use crate::history::Source;
use std::sync::Arc;

fn setup() -> (tempfile::TempDir, Arc<Core>, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let outer = temp.path().join("02_Тексты_и_книги");
    let inner = outer.join("05_Папки_книг").join("Английский");
    fs::create_dir_all(&inner).unwrap();
    let outer = canonical_path(&outer);
    let inner = outer.join("05_Папки_книг").join("Английский");
    let data = temp.path().join("appdata");
    fs::create_dir_all(&data).unwrap();
    let core = Core::open(&data, Arc::new(|_: CoreEvent| {}));
    core.ui_state.resolve_workspace(outer.to_str().unwrap(), 1).unwrap();
    core.ui_state.resolve_workspace(inner.to_str().unwrap(), 2).unwrap();
    (temp, core, outer, inner)
}

#[test]
fn unrelated_quantum_archive_does_not_block_new_note_rename() {
    let (_temp, core, outer, inner) = setup();
    let unrelated = outer.join(".quantum").join("history").join("Старая заметка.md");
    fs::create_dir_all(&unrelated).unwrap();
    fs::write(unrelated.join("old_snapshot.md"), "untouched").unwrap();
    let old = inner.join("Без названия.md");
    let new = inner.join("Втекст.md");
    crate::files::gate::create(&core, &old, "Мой английский", Source::Me, None).unwrap();
    crate::files::gate::rename(&core, &old, &new).unwrap();
    assert_eq!(fs::read_to_string(&new).unwrap(), "Мой английский");
    assert_eq!(fs::read_to_string(unrelated.join("old_snapshot.md")).unwrap(), "untouched");
    assert!(core.history.page(&|| core.known_roots(), &new, 0, 8).total > 0);
}

#[test]
fn independent_history_in_both_registered_roots_stays_fail_closed() {
    let (_temp, core, outer, inner) = setup();
    let old = inner.join("Без названия.md");
    let new = inner.join("Втекст.md");
    crate::files::gate::create(&core, &old, "keep these bytes", Source::Me, None).unwrap();
    let separate = crate::history::store::history_dir(
        &outer, old.strip_prefix(&outer).unwrap());
    fs::create_dir_all(&separate).unwrap();
    fs::write(separate.join("independent_snapshot.md"), "second history").unwrap();
    assert!(crate::files::gate::rename(&core, &old, &new).is_err());
    assert_eq!(fs::read_to_string(&old).unwrap(), "keep these bytes");
    assert_eq!(fs::read_to_string(separate.join("independent_snapshot.md")).unwrap(), "second history");
    assert!(!new.exists());
}

#[test]
fn preexisting_destination_history_is_not_overwritten() {
    let (_temp, core, outer, inner) = setup();
    let old = inner.join("Без названия.md");
    let new = inner.join("Втекст.md");
    crate::files::gate::create(&core, &old, "keep source", Source::Me, None).unwrap();
    let archive = crate::history::store::history_dir(
        &outer, new.strip_prefix(&outer).unwrap());
    fs::create_dir_all(&archive).unwrap();
    fs::write(archive.join("old_target.md"), "do not merge").unwrap();
    assert!(crate::files::gate::rename(&core, &old, &new).is_err());
    assert_eq!(fs::read_to_string(&old).unwrap(), "keep source");
    assert_eq!(fs::read_to_string(archive.join("old_target.md")).unwrap(), "do not merge");
}

#[test]
fn this_notes_own_quantum_history_must_not_be_stranded() {
    let (_temp, core, outer, inner) = setup();
    let old = inner.join("Без названия.md");
    let new = inner.join("Втекст.md");
    fs::write(&old, "keep source").unwrap();
    let legacy = outer.join(".quantum").join("history")
        .join(old.strip_prefix(&outer).unwrap());
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("1_snapshot.md"), "old version").unwrap();
    assert!(crate::files::gate::rename(&core, &old, &new).is_err());
    assert_eq!(fs::read_to_string(&old).unwrap(), "keep source");
    assert_eq!(fs::read_to_string(legacy.join("1_snapshot.md")).unwrap(), "old version");
}
