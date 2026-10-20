use super::*;

fn version_with_large_diff(h: &mut Harness, path: &std::path::Path) -> crate::history::Version {
    use aquilum_core::files::gate;
    use aquilum_core::history::Source;
    use aquilum_core::files::document::read_file_snapshot_impl;

    let replacement = "Версия с большой историей.\n".repeat(1_000);
    let hash = read_file_snapshot_impl(path).unwrap().hash;
    // Seed a real large disk-backed history entry *before* opening the note.
    // Writing externally while the editor is open could instead exercise
    // stale-document conflict handling rather than the view's memory lifetime.
    gate::write(&h.app.workspace.core, path, &replacement, Some(&hash), Source::Me, None).unwrap();
    crate::history::page(&h.app.workspace.core, path, crate::history::PAGE_SIZE)
        .versions
        .into_iter()
        .find(|version| {
            crate::history::texts(&h.app.workspace.core, path, &version.id)
                .is_some_and(|texts| texts.text == replacement)
        })
        .expect("history contains the 1,000-line diff, not the short old version")
}

#[test]
fn closing_version_discards_hidden_widget_tree_but_not_disk_history() {
    let mut h = Harness::new();
    let path = h.write_note("large-history.md", "Первоначальный текст.\n");
    let version = version_with_large_diff(&mut h, &path);
    assert!(h.app.open(&mut h.root, path.clone()));
    h.app.open_version(&mut h.root, version.clone());
    let widget = h.app.version_view.as_ref().expect("version open").back;
    assert!(h.root.get_widget(widget).is_some());
    h.app.close_version(&mut h.root);
    let _ = h.root.redraw();
    assert!(h.app.version_view.is_none());
    assert!(h.root.get_widget(widget).is_none(), "closed history controls remain mounted");
    assert!(!h.app.opened_versions.contains_key(&h.app.tabs.active_id()));
    assert!(crate::history::texts(&h.app.workspace.core, &path, &version.id).is_some(), "disk history must survive UI eviction");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "Версия с большой историей.\n".repeat(1_000));
}

#[test]
fn tab_switch_evicts_inactive_diff_and_restores_history_on_return() {
    let mut h = Harness::new();
    let first = h.write_note("versioned.md", "Первоначальный текст.\n");
    let second = h.write_note("other.md", "Другая заметка.\n");
    let version = version_with_large_diff(&mut h, &first);
    assert!(h.app.open(&mut h.root, first.clone()));
    h.app.open_version(&mut h.root, version);
    let old = h.app.version_view.as_ref().expect("diff mounted").back;
    let first_tab = h.app.tabs.active_id();
    assert!(h.app.open_in(&mut h.root, second, true));
    let _ = h.root.redraw();
    assert!(h.root.get_widget(old).is_none(), "inactive diff widget is retained");
    assert!(h.app.version_view.is_none());
    assert!(h.app.opened_versions.contains_key(&first_tab), "selected version identity must be kept");
    assert!(h.app.tabs.select(first_tab));
    h.app.show_active(&mut h.root);
    let _ = h.root.redraw();
    let restored = h.app.version_view.as_ref().expect("diff reconstructed").back;
    assert_ne!(restored, old);
    assert!(h.root.get_widget(restored).is_some());
    assert_eq!(std::fs::read_to_string(&first).unwrap(), "Версия с большой историей.\n".repeat(1_000));
}
