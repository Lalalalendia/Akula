use super::*;

impl Harness {
    pub(super) fn type_search(&mut self, text: &str) {
        let field = match &self.app.popup {
            Some(Popup::Search(state)) => state.field_id(),
            _ => panic!("поиск не открыт"),
        };
        self.send(field, crate::ui::editor::TextAction::Changed(text.into()));
    }

    pub(super) fn wait_results(&mut self, at_least: usize) -> usize {
        let count = |h: &Harness| match &h.app.popup {
            Some(Popup::Search(state)) => state.result_count(),
            _ => 0,
        };
        let found = wait_until(250, 20, || {
            self.app.on_core_events(&mut self.root);
            count(self) >= at_least
        });
        assert!(found, "поиск не дал результатов");
        count(self)
    }
}

#[test]
fn search_finds_opens_and_creates_notes() {
    let mut h = Harness::new();
    let vault = h.vault.path().to_path_buf();
    h.write_note("Кофе.md", "Заметка про обжарку кофе");
    h.write_note("Чай.md", "Кофе тоже упомянут");
    h.app.start_index();
    wait_until(250, 20, || {
        let status = h.app.workspace.core.search.status(Some(&vault.to_string_lossy()));
        status.state == aquilum_core::search::models::SearchIndexState::Ready && !status.updating && status.indexed_documents >= 3
    });
    h.app.open_search(&mut h.root);
    h.type_search("кофе");
    assert_eq!(h.wait_results(2), 2);
    h.app.search_key(&mut h.root, &Key::Named(NamedKey::ArrowDown), Modifiers::empty());
    let second = match &h.app.popup {
        Some(Popup::Search(state)) => state.selected_path(),
        _ => unreachable!(),
    };
    h.app.search_key(&mut h.root, &Key::Named(NamedKey::Enter), Modifiers::empty());
    assert!(h.app.popup.is_none());
    assert_eq!(h.app.tabs.active().path.as_ref(), Some(&second));

    h.app.open_search(&mut h.root);
    h.type_search("Новая идея");
    h.app.search_key(&mut h.root, &Key::Named(NamedKey::Enter), Modifiers::SHIFT);
    assert!(vault.join("Новая идея.md").exists());
    assert_eq!(h.app.tabs.active().path.as_ref(), Some(&vault.join("Новая идея.md")));
}

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
    let (selected, snippet) = match &h.app.popup {
        Some(Popup::Search(state)) => (
            state.selected_path(),
            state.selected_snippet().to_owned(),
        ),
        _ => panic!("search must remain open until the result is chosen"),
    };
    assert_eq!(selected, expected, "search pointed to a different document");
    assert!(snippet.contains(MARKER), "user-visible search snippet lost the source phrase");

    h.app.search_key(&mut h.root, &Key::Named(NamedKey::Enter), Modifiers::empty());
    assert!(h.app.popup.is_none(), "opening a search result must close search");
    assert_eq!(h.app.tabs.active().path.as_ref(), Some(&expected));
    assert!(h.body_text().contains(MARKER), "reopened native editor lost the source text");
    assert_eq!(std::fs::read(&expected).unwrap(), saved_bytes, "search/open mutated the note");
    eprintln!(
        "AKULA_FIRST_LOCAL_JOURNEY_PASS=gui_create,save,restart,index,search_snippet,open;source_unchanged=true"
    );
}
