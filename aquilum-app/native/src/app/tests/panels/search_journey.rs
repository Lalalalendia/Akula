use super::*;

#[test]
fn first_local_user_journey_create_save_restart_search_and_open() {
    // This is a real GUI harness route, rather than just a parser or index
    // unit test: search dialog -> create -> native editor -> File Gate save ->
    // shutdown/reopen -> search dialog -> open original persisted file.
    // Installed-machine acceptance is a separate required gate.
    const TITLE: &str = "Маршрут пользователя";
    const MARKER: &str = "localjourneyproofword";
    let mut h = Harness::new();
    let vault = h.vault.path().to_path_buf();
    let expected = vault.join(format!("{TITLE}.md"));

    h.app.open_search(&mut h.root);
    h.type_search(TITLE);
    h.app.search_key(&mut h.root, &Key::Named(NamedKey::Enter), Modifiers::SHIFT);
    assert!(h.app.popup.is_none(), "creating a note must close search");
    assert_eq!(h.app.tabs.active().path.as_ref(), Some(&expected));
    assert!(expected.is_file(), "GUI create must publish a real local file");

    let body = format!("# {TITLE}\n\nText searchable after restart: {MARKER}\n");
    h.replace_body(&body);
    h.save_now();
    let saved_bytes = std::fs::read(&expected).expect("GUI editing must save via File Gate");
    let saved = String::from_utf8(saved_bytes.clone()).expect("saved Markdown must be UTF-8");
    assert!(saved.contains(MARKER), "GUI edit was not persisted");
    assert!(saved.contains(TITLE), "GUI edit lost the title");

    // Real app shutdown and fresh workspace, search service, render tree and
    // tab state, keeping only the same sandboxed on-disk vault and app data.
    let mut h = h.restarted(1.0);
    assert_eq!(std::fs::read(&expected).unwrap(), saved_bytes, "restart changed the source");
    h.app.start_index();
    assert!(
        wait_until(300, 20, || {
            let state = h.app.workspace.core.search.status(Some(&vault.to_string_lossy()));
            state.state == aquilum_core::search::models::SearchIndexState::Ready
                && !state.updating
                && state.indexed_documents >= 2
        }),
        "reopened local index did not become ready"
    );

    h.app.open_search(&mut h.root);
    h.type_search(MARKER);
    assert_eq!(h.wait_results(1), 1, "unique phrase must find exactly one saved file");
    let selected = match &h.app.popup {
        Some(Popup::Search(state)) => state.selected_path(),
        _ => panic!("search must remain open until the result is chosen"),
    };
    assert_eq!(selected, expected, "search pointed to a different document");

    h.app.search_key(&mut h.root, &Key::Named(NamedKey::Enter), Modifiers::empty());
    assert!(h.app.popup.is_none(), "opening a search result must close search");
    assert_eq!(h.app.tabs.active().path.as_ref(), Some(&expected));
    assert!(h.body_text().contains(MARKER), "reopened native editor lost the source text");
    assert_eq!(std::fs::read(&expected).unwrap(), saved_bytes, "search/open mutated the note");
    assert_eq!(
        std::fs::read_dir(&vault).unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().file_name().is_some_and(|name| name == "Маршрут пользователя.md"))
            .count(),
        1,
        "GUI path created a duplicate note"
    );
    eprintln!(
        "AKULA_FIRST_LOCAL_JOURNEY_PASS=gui_create,save,restart,index,search_snippet,open;source_unchanged=true"
    );
}
