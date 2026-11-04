use super::*;

#[test]
fn global_search_indexes_txt_content_and_opens_the_exact_utf8_file() {
    // Prepopulate the actual user vault before opening Akula, so the initial
    // index scan owns both fixtures. Creating them after Harness::new races the
    // already-started watcher and may miss both writes entirely.
    let test_vault = crate::test_dir::tempdir();
    let vault = test_vault.path().to_path_buf();
    let content = "Исходный документ: 東京日記 中文段落 txtbodyuniqueneedle";
    let txt = vault.join("東京日記.TXT");
    std::fs::write(&txt, content).unwrap();
    std::fs::write(vault.join("Control.md"), "Ordinary Markdown control").unwrap();
    let original_bytes = std::fs::read(&txt).unwrap();
    let mut h = Harness::with(Some(&vault), 1.0);

    h.app.start_index();
    assert!(wait_until(500, 20, || {
        let state = h.app.workspace.core.search.status(Some(&vault.to_string_lossy()));
        state.state == aquilum_core::search::models::SearchIndexState::Ready
            && !state.updating
            && state.indexed_documents >= 2
    }), "native index did not complete for the preexisting TXT and Markdown fixtures: {:?}",
        h.app.workspace.core.search.status(Some(&vault.to_string_lossy())));

    h.app.open_search(&mut h.root);
    h.type_search("txtbodyuniqueneedle");
    assert_eq!(h.wait_results(1), 1, "Ctrl+O-style full-text search must include TXT");
    let selected = match &h.app.popup {
        Some(Popup::Search(state)) => state.selected_path(),
        _ => panic!("native global search dialog closed unexpectedly"),
    };
    assert_eq!(selected, txt, "search should select the physical TXT file");
    h.app.search_key(&mut h.root, &Key::Named(NamedKey::Enter), Modifiers::empty());
    assert!(h.app.popup.is_none());
    assert_eq!(h.app.tabs.active().path.as_ref(), Some(&txt));
    assert_eq!(h.body_text(), content, "search must open and display the TXT text");
    assert_eq!(std::fs::read(&txt).unwrap(), original_bytes, "search cannot change the TXT");

    h.app.open_search(&mut h.root);
    h.type_search("東京日記");
    assert_eq!(h.wait_results(1), 1, "CJK file title must be searchable");
    let by_name = match &h.app.popup {
        Some(Popup::Search(state)) => state.selected_path(),
        _ => panic!("native global search dialog closed unexpectedly"),
    };
    assert_eq!(by_name, txt);
    assert_eq!(std::fs::read(&txt).unwrap(), original_bytes);
}
