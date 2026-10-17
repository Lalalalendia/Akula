use super::*;

fn select(h: &mut Harness, path: &std::path::Path) -> crate::app::tabs::open::TabSwitchStages {
    let id = h.app.tabs.tabs().iter().find(|tab| tab.path.as_deref() == Some(path)).expect("tab exists").id;
    h.app.on_titlebar(&mut h.root, &crate::ui::titlebar::TitlebarAction::Select(id), Size::new(f64::from(W), f64::from(H)));
    crate::app::tabs::open::take_tab_switch_stages()
}

#[test]
fn clean_tabs_reopen_from_bounded_rope_cache_after_byte_hash_verification() {
    let mut h = Harness::new();
    let first = h.write_note("cached-A.md", "A: текст");
    let second = h.write_note("cached-B.md", "B: текст");
    assert!(h.app.open_in(&mut h.root, first.clone(), true));
    assert!(h.app.open_in(&mut h.root, second.clone(), true));

    assert_eq!(h.app.inactive_notes.len(), 1);
    let stages = select(&mut h, &first);
    assert!(stages.cache_hit, "clean tab may reuse Rope after disk hash matches");
    assert_eq!(h.body_text(), "A: текст");
    let stages = select(&mut h, &second);
    assert!(stages.cache_hit);
    assert_eq!(h.body_text(), "B: текст");
}

#[test]
fn cached_tab_rejects_same_length_same_mtime_external_changes() {
    let mut h = Harness::new();
    let first = h.write_note("same-stat.md", "раз");
    let second = h.write_note("other.md", "спокойно");
    assert!(h.app.open_in(&mut h.root, first.clone(), true));
    assert!(h.app.open_in(&mut h.root, second.clone(), true));
    assert_eq!(h.app.inactive_notes.len(), 1);

    let metadata = std::fs::metadata(&first).unwrap();
    let when = metadata.modified().unwrap();
    std::fs::write(&first, "два").unwrap();
    std::fs::File::open(&first).unwrap().set_modified(when).unwrap();
    assert_eq!(metadata.len(), std::fs::metadata(&first).unwrap().len());
    assert_eq!(when, std::fs::metadata(&first).unwrap().modified().unwrap());

    let stages = select(&mut h, &first);
    assert!(!stages.cache_hit, "changed raw bytes must invalidate cached Rope");
    assert_eq!(h.body_text(), "два");
}

#[test]
fn pending_local_edits_are_never_cached_and_survive_fast_tab_switches() {
    let mut h = Harness::new();
    let first = h.write_note("draft.md", "начало");
    let second = h.write_note("other.md", "второй");
    assert!(h.app.open_in(&mut h.root, first.clone(), true));
    h.replace_body("локальная версия");
    assert!(h.app.note.as_ref().is_some_and(|n| n.deadline().is_some()));
    assert!(h.app.open_in(&mut h.root, second.clone(), true));
    assert!(h.app.inactive_notes.is_empty(), "pending edits must not become clean cache");
    crate::disk::wait();
    assert_eq!(std::fs::read_to_string(&first).unwrap(), "локальная версия");

    let stages = select(&mut h, &first);
    assert!(!stages.cache_hit, "unclean tab must be loaded from disk");
    assert_eq!(h.body_text(), "локальная версия");
}

#[test]
fn closing_tabs_evicts_private_snapshots_and_count_is_bounded() {
    let mut h = Harness::new();
    let paths: Vec<_> = (0..4).map(|n| h.write_note(&format!("tab-{n}.md"), &format!("заметка {n}"))).collect();
    for path in &paths {
        assert!(h.app.open_in(&mut h.root, path.clone(), true));
    }
    assert_eq!(h.app.inactive_notes.len(), 2, "cache holds at most two inactive tabs");

    let cached = &paths[1];
    let closed = h.app.tabs.tabs().iter().find(|tab| tab.path.as_deref() == Some(cached.as_path())).unwrap().id;
    h.app.on_titlebar(&mut h.root, &crate::ui::titlebar::TitlebarAction::Close(closed), Size::new(f64::from(W), f64::from(H)));
    assert_eq!(h.app.inactive_notes.len(), 1, "closed tab must release its snapshot");
}

#[test]
fn deleted_file_never_reopens_stale_cached_bytes() {
    let mut h = Harness::new();
    let first = h.write_note("later-deleted.md", "старый текст");
    let second = h.write_note("still-open.md", "текст");
    assert!(h.app.open_in(&mut h.root, first.clone(), true));
    assert!(h.app.open_in(&mut h.root, second, true));
    assert_eq!(h.app.inactive_notes.len(), 1);
    std::fs::remove_file(&first).unwrap();

    let stages = select(&mut h, &first);
    assert!(!stages.cache_hit);
    assert!(h.app.open_error.is_some(), "missing file must surface open error, not cached text");
}

#[test]
fn parley_trial_rejects_selected_text_and_secondary_carets() {
    let mut h = Harness::new();
    h.app.parley_trial_enabled = true;
    let text = "Обычный абзац без ссылок или украшений.\n".repeat(120);
    let path = h.write_note("parley-multicaret.md", &text);
    assert!(h.app.open_in(&mut h.root, path.clone(), true));
    let _ = h.root.redraw();

    let body = h.app.note_ids.as_ref().expect("active editor").body;
    assert!(
        body.get(&h.root).and_then(|editor| editor.parley_trial_snapshot()).is_some(),
        "control: a clean, already-laid-out plain note should be eligible"
    );

    body.edit(&mut h.root, |mut editor| {
        crate::ui::editor::TextEditor::select_byte_range(&mut editor, 0, 6);
    });
    assert!(
        body.get(&h.root).and_then(|editor| editor.parley_trial_snapshot()).is_none(),
        "a highlighted selection must never enter the layout cache"
    );

    body.edit(&mut h.root, |mut editor| {
        crate::ui::editor::TextEditor::select_byte_range(&mut editor, 6, 6);
        crate::ui::editor::TextEditor::add_secondary_caret_for_trial_test(&mut editor, 12);
    });
    assert!(
        body.get(&h.root).and_then(|editor| editor.parley_trial_snapshot()).is_none(),
        "even a zero-width secondary caret must invalidate the layout snapshot"
    );
    assert_eq!(h.body_text(), text, "rejecting cached layouts must not mutate source text");
    h.app.shutdown();
}
