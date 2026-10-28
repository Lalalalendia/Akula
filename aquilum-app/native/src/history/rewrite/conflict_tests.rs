use std::cell::Cell;
use std::fs;
use std::sync::Arc;

use aquilum_core::Core;
use aquilum_core::files::error::FileCommandError;
use aquilum_core::history::Source;

use super::rewrite;

#[test]
fn restore_rejects_a_concurrent_disk_edit_without_overwriting_it() {
    let vault = crate::test_dir::tempdir();
    let data = crate::test_dir::tempdir();
    let core = Core::open(data.path(), Arc::new(|_| {}));
    core.ui_state.resolve_workspace(&vault.path().to_string_lossy(), 0).unwrap();
    let note = vault.path().join("restore-conflict.md");
    fs::write(&note, "newer user content").unwrap();

    let attempts = Cell::new(0);
    let result = rewrite(&core, &note, Source::Restore(1), false, |current| {
        attempts.set(attempts.get() + 1);
        assert_eq!(current, "newer user content");
        fs::write(&note, "concurrent external change").unwrap();
        "historical snapshot".to_owned()
    });

    assert!(matches!(&result, Err(FileCommandError::Conflict { .. })), "{result:?}");
    assert_eq!(attempts.get(), 1, "Restore must not retry with a newer hash");
    assert_eq!(fs::read_to_string(&note).unwrap(), "concurrent external change");
    core.shutdown();
}

#[test]
fn revert_still_recomputes_against_a_fresh_snapshot_on_conflict() {
    let vault = crate::test_dir::tempdir();
    let data = crate::test_dir::tempdir();
    let core = Core::open(data.path(), Arc::new(|_| {}));
    core.ui_state.resolve_workspace(&vault.path().to_string_lossy(), 0).unwrap();
    let note = vault.path().join("revert-conflict.md");
    fs::write(&note, "initial").unwrap();

    let attempts = Cell::new(0);
    let result = rewrite(&core, &note, Source::Revert(1), true, |current| {
        let prior = attempts.get();
        attempts.set(prior + 1);
        if prior == 0 {
            fs::write(&note, "external addition").unwrap();
        }
        format!("{current} + reverted")
    });

    assert!(matches!(&result, Ok(true)), "{result:?}");
    assert_eq!(attempts.get(), 2, "Revert must reconsider fresh disk content");
    assert_eq!(fs::read_to_string(&note).unwrap(), "external addition + reverted");
    core.shutdown();
}
