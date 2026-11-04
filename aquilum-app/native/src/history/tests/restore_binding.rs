//! A snapshot selected in the UI must never overwrite bytes unseen by the editor.

use aquilum_core::Core;
use aquilum_core::files::document::read_file_snapshot_impl;
use aquilum_core::files::error::FileCommandError;
use aquilum_core::files::gate;
use aquilum_core::history::Source;

use crate::history::{PAGE_SIZE, page, restore, texts};

#[test]
fn restore_rejects_external_edit_since_last_clean_editor_snapshot() {
    let vault = crate::test_dir::tempdir();
    let data = crate::test_dir::tempdir();
    let core = Core::open(data.path(), std::sync::Arc::new(|_| {}));
    core.ui_state.resolve_workspace(&vault.path().to_string_lossy(), 0).unwrap();
    let note = vault.path().join("restore-stale.md");
    std::fs::write(&note, "original").unwrap();
    let initial_hash = read_file_snapshot_impl(&note).unwrap().hash;
    gate::write(&core, &note, "saved by Akula", Some(&initial_hash), Source::Me, None).unwrap();
    let versions = page(&core, &note, PAGE_SIZE).versions;
    let oldest = versions.iter().find(|v|
        texts(&core, &note, &v.id).is_some_and(|t| t.text == "original")
    ).expect("baseline history version exists");
    let editor_hash = read_file_snapshot_impl(&note).unwrap().hash;
    std::fs::write(&note, "new external edits").unwrap();
    let before = page(&core, &note, PAGE_SIZE).total;

    let result = restore(&core, &note, oldest, &editor_hash);
    assert!(matches!(&result, Err(FileCommandError::Conflict { .. })), "{result:?}");
    assert_eq!(std::fs::read_to_string(&note).unwrap(), "new external edits");
    assert_eq!(page(&core, &note, PAGE_SIZE).total, before,
        "rejected Restore must not create a history version");
    core.shutdown();
}

#[test]
fn restore_refuses_stale_editor_hash_even_when_disk_already_matches_target() {
    let vault = crate::test_dir::tempdir();
    let data = crate::test_dir::tempdir();
    let core = Core::open(data.path(), std::sync::Arc::new(|_| {}));
    core.ui_state.resolve_workspace(&vault.path().to_string_lossy(), 0).unwrap();
    let note = vault.path().join("restore-stale-noop.md");
    std::fs::write(&note, "first").unwrap();
    let hash = read_file_snapshot_impl(&note).unwrap().hash;
    gate::write(&core, &note, "second", Some(&hash), Source::Me, None).unwrap();
    let oldest = page(&core, &note, PAGE_SIZE).versions.into_iter().find(|v|
        texts(&core, &note, &v.id).is_some_and(|t| t.text == "first")
    ).unwrap();
    let editor_hash = read_file_snapshot_impl(&note).unwrap().hash;
    std::fs::write(&note, "first").unwrap();

    let result = restore(&core, &note, &oldest, &editor_hash);
    assert!(matches!(&result, Err(FileCommandError::Conflict { .. })), "{result:?}");
    assert_eq!(std::fs::read_to_string(&note).unwrap(), "first");
    core.shutdown();
}
