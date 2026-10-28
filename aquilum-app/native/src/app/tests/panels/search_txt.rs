use super::*;

#[test]
fn global_search_indexes_txt_content_and_opens_the_exact_utf8_file() {
    let mut h = Harness::new();
    let vault = h.vault.path().to_path_buf();
    let content = "Исходный документ: 東京日記 中文段落 txtbodyuniqueneedle";
    let txt = h.write_note("東京日記.TXT", content);
    let original_bytes = std::fs::read(&txt).unwrap();

    h.app.start_index();
    assert!(wait_until(250, 20, || {
        let state = h.app.workspace.core.search.status(Some(&vault.to_string_lossy()));
        state.state == aquilum_core::search::models::SearchIndexState::Ready
            && !state.updating
            && state.indexed_documents >= 2
    }), "native search did not index the UTF-8 TXT");

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
