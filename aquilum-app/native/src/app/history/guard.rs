//! A history operation must not replace a note while its current editor
//! revision is unsaved, conflicted, or awaiting a save acknowledgement.

use std::sync::Arc;

use aquilum_core::Core;

use crate::note::Note;

pub(super) fn flush_clean(note: &mut Note, core: &Arc<Core>, text: &crop::Rope) -> bool {
    note.flush(core, text);
    note.clean_cache_hash().is_some()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn a_conflicting_editor_save_blocks_history_restore_before_disk_rewrite() {
        let vault = crate::test_dir::tempdir();
        let data = crate::test_dir::tempdir();
        let core = Core::open(data.path(), Arc::new(|_| {}));
        core.ui_state.resolve_workspace(&vault.path().to_string_lossy(), 0).unwrap();
        let path = vault.path().join("history-guard.md");
        std::fs::write(&path, "original").unwrap();
        let (mut note, _) = Note::open(path.clone()).unwrap();
        note.edit(Duration::ZERO);
        std::fs::write(&path, "external edit").unwrap();
        let unsaved = crop::Rope::from("new editor text");

        assert!(!flush_clean(&mut note, &core, &unsaved));
        assert!(note.save_error.is_some(), "save conflict must remain visible");
        assert_eq!(std::fs::read_to_string(path).unwrap(), "external edit");
        core.shutdown();
    }

    #[test]
    fn a_successful_flush_allows_history_operation_without_dropping_text() {
        let vault = crate::test_dir::tempdir();
        let data = crate::test_dir::tempdir();
        let core = Core::open(data.path(), Arc::new(|_| {}));
        core.ui_state.resolve_workspace(&vault.path().to_string_lossy(), 0).unwrap();
        let path = vault.path().join("history-ready.md");
        std::fs::write(&path, "original").unwrap();
        let (mut note, _) = Note::open(path.clone()).unwrap();
        note.edit(Duration::ZERO);
        let latest = crop::Rope::from("saved editor text");

        assert!(flush_clean(&mut note, &core, &latest));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "saved editor text");
        core.shutdown();
    }
}
