use super::*;
#[test]
fn book_quote_survives_restart_is_searchable_and_opens_the_source() {
// An end-to-end native GUI workflow, rather than an isolated CFI or
    // quote parser test: reading -> visible selection -> quote action ->
    // durable Markdown -> process restart -> user search -> open the
    // original note -> follow the quote link back to the book.
    let mut h = Harness::new();
    let vault = h.vault.path().to_path_buf();
    let (book, note) = book_note(&h, "Заметки о книге");
    let original_book = std::fs::read(&book).expect("read original fixture");
    assert!(h.app.open(&mut h.root, note.clone()));
    let _ = h.root.redraw();
    open_book(&mut h, &book, Some(&note), "Книга");
    assert!(h.reader());
    quote_selection(&mut h, (120.0, 160.0), (420.0, 220.0));
    h.save_now();

    let saved_note = std::fs::read(&note).expect("quote was not saved");
    let persisted = String::from_utf8(saved_note.clone()).expect("note must be UTF-8");
    let quotes = crate::reader::quotes::find(&persisted);
    assert_eq!(quotes.len(), 1, "one user selection must produce one quote");
    assert!(persisted[quotes[0].text.clone()].contains("Абзац номер"));
    assert_eq!(quotes[0].book.as_deref(), Some("Files/книга.fb2"));
    let cfi = quotes[0].cfi.clone();
    assert!(crate::reader::Book::open(&book)
        .and_then(|source| source.resolve(&cfi)).is_some(), "persisted CFI cannot resolve");
    assert_eq!(std::fs::read(&book).unwrap(), original_book, "quote modified book source");

    h.escape();
    assert!(!h.reader(), "reader must close before app restart");
    let mut h = h.restarted(1.0);
    assert_eq!(std::fs::read(&note).unwrap(), saved_note, "restart modified note");
    h.app.start_index();
    assert!(wait_until(300, 20, || {
        let state = h.app.workspace.core.search.status(Some(&vault.to_string_lossy()));
        state.state == aquilum_core::search::models::SearchIndexState::Ready
            && !state.updating && state.indexed_documents >= 2
    }), "local index did not become ready after restart");

    // Type into Akula's actual native search dialog, then open the result.
    h.app.open_search(&mut h.root);
    let search_field = match &h.app.popup {
        Some(Popup::Search(state)) => state.field_id(),
        _ => panic!("search dialog did not open"),
    };
    h.send(search_field, crate::ui::editor::TextAction::Changed("Абзац номер".into()));
    assert!(wait_until(300, 20, || {
        h.app.on_core_events(&mut h.root);
        matches!(&h.app.popup, Some(Popup::Search(state)) if state.result_count() == 1)
    }), "saved book quote not found by user search");
    let selected = match &h.app.popup {
        Some(Popup::Search(state)) => state.selected_path(),
        _ => panic!("search unexpectedly closed"),
    };
    assert_eq!(selected, note, "quote search targeted the wrong document");
    h.app.search_key(&mut h.root, &Key::Named(NamedKey::Enter), Modifiers::empty());
    assert!(h.app.popup.is_none(), "search did not close after opening note");
    assert_eq!(h.app.tabs.active().path.as_ref(), Some(&note));
    let reopened_text = h.body_text();
    let restored = crate::reader::quotes::find(&reopened_text);
    assert_eq!(restored.len(), 1, "quote was lost or duplicated on reload");
    assert_eq!(restored[0].cfi, cfi, "quote target moved after restart");

    let quote_url = format!(
        "aquilum-reader:cfi={}",
        cfi.replace('(', "%28").replace(')', "%29"),
    );
    h.app.open_target(&mut h.root, crate::links::Target::Url(quote_url), false);
    h.deliver();
    let _ = h.root.redraw();
    assert!(h.reader(), "link in saved quote cannot reopen its book");
    assert_eq!(std::fs::read(&book).unwrap(), original_book, "opening quote changed source");
    assert_eq!(std::fs::read(&note).unwrap(), saved_note, "search or link changed saved note");
    eprintln!("AKULA_BOOK_QUOTE_JOURNEY_PASS=reader,quote,save,restart,search,note,source_link;originals_unchanged=true");
}
