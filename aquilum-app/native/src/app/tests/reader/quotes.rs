use super::*;
use crate::ui::reader::ReaderView;
use masonry::kurbo::Rect;

fn quote_selection(h: &mut Harness, from: (f64, f64), to: (f64, f64)) {
    h.drag(from, to);
    let _ = h.root.redraw();
    h.deliver();
    let quote = match &h.app.popup {
        Some(Popup::Reader(state)) => state.ids.quote.unwrap(),
        _ => panic!("читалка закрылась"),
    };
    let button = h.root.get_widget(quote).unwrap().ctx().bounding_box();
    assert!(button.width() > 0.0, "нет кнопки цитаты");
    h.click(button.center().x, button.center().y);
    let _ = h.root.redraw();
    h.deliver();
}

fn quote_boxes(h: &mut Harness) -> Vec<(String, Vec<Rect>)> {
    let _ = h.root.redraw();
    match &h.app.popup {
        Some(Popup::Reader(state)) => h.root.get_widget(state.layer).and_then(|w| w.downcast::<ReaderView>().map(|v| v.inner().quote_boxes())).unwrap(),
        _ => panic!("читалка закрылась"),
    }
}

#[test]
fn reader_quotes_round_trip() {
    let mut h = Harness::new();
    let (file, page) = book_note(&h, "Заметки о книге");
    h.app.open(&mut h.root, page.clone());
    let _ = h.root.redraw();
    open_book(&mut h, &file, Some(&page), "Книга");
    quote_selection(&mut h, (120.0, 160.0), (420.0, 220.0));
    let text = h.app.text.to_string();
    let found = crate::reader::quotes::find(&text);
    assert_eq!(found.len(), 1, "{text}");
    assert_eq!(found[0].label, "1");
    assert_eq!(found[0].book.as_deref(), Some("Files/книга.fb2"));
    assert!(found[0].cfi.starts_with("epubcfi(/6/2!"), "{}", found[0].cfi);
    assert!(text[found[0].text.clone()].contains("Абзац номер"), "{text}");
    h.save_now();
    assert!(std::fs::read_to_string(&page).unwrap().contains("[!quote]"), "цитата не сохранена в файл");
    let _ = h.root.redraw();
    let label = match &h.app.popup {
        Some(Popup::Reader(state)) => h.root.get_widget(state.layer).and_then(|w| w.downcast::<ReaderView>().map(|v| v.inner().label_rects())).unwrap(),
        _ => panic!("читалка закрылась"),
    };
    assert_eq!(label.len(), 1, "метка цитаты не нарисована");
    h.click(label[0].center().x, label[0].center().y);
    assert!(!h.reader(), "щелчок по метке не закрыл читалку");
    let selected = h.root.get_widget(h.app.editor_id().unwrap()).and_then(|w| w.downcast::<crate::ui::editor::TextEditor>()).unwrap().inner().selected();
    assert_eq!(selected, text[found[0].text.clone()]);
    let url = format!("aquilum-reader:cfi={}", found[0].cfi.replace('(', "%28").replace(')', "%29"));
    h.app.open_target(&mut h.root, crate::links::Target::Url(url), false);
    h.deliver();
    let _ = h.root.redraw();
    assert!(h.reader(), "ссылка на цитату не открыла книгу");
}

fn two_quotes(h: &mut Harness) {
    let (file, page) = book_note(h, "Заметки о книге");
    h.app.open(&mut h.root, page.clone());
    let _ = h.root.redraw();
    open_book(h, &file, Some(&page), "Книга");
    quote_selection(h, (300.0, 150.0), (500.0, 330.0));
    quote_selection(h, (200.0, 380.0), (600.0, 470.0));
}

fn distance(shot: &crate::render::Renderer, x: f64, y: f64) -> i32 {
    let p = shot.frame_pixels()[y as usize * W as usize + x as usize];
    let bg = crate::ui::theme::background().to_rgba8();
    [((p >> 16) as u8, bg.r), ((p >> 8) as u8, bg.g), (p as u8, bg.b)].iter().map(|(a, b)| i32::from(*a).abs_diff(i32::from(*b)) as i32).sum()
}

#[test]
fn a_quote_hugs_its_lines_without_gaps_or_trailing_spaces() {
    let mut h = Harness::new();
    two_quotes(&mut h);
    let boxes = quote_boxes(&mut h);
    let (_, rects) = &boxes[0];
    let body = &rects[1..];
    assert!(body.len() >= 4, "{rects:?}");
    let right = body.iter().map(|r| r.x1).fold(f64::MIN, f64::max);
    let full = body.iter().filter(|r| (r.x1 - right).abs() < 0.01).count();
    assert!(full >= 3, "строки на всю ширину колонки не выровнены по правому краю: {body:?}");
    assert!(body.iter().any(|r| r.x1 < right - 20.0), "конец абзаца закрашен до края колонки, а не по тексту: {body:?}");
    assert!(body.windows(2).all(|w| w[1].y0 == w[0].y1), "между строками цитаты зазоры или наложения: {body:?}");
}

#[test]
fn the_quote_you_came_from_is_brightest_and_hovering_deepens_the_others() {
    let mut h = Harness::new();
    two_quotes(&mut h);
    h.escape();
    let found = crate::reader::quotes::find(&h.app.text.to_string());
    let url = format!("aquilum-reader:cfi={}", found[1].cfi.replace('(', "%28").replace(')', "%29"));
    h.app.open_target(&mut h.root, crate::links::Target::Url(url), false);
    h.deliver();
    h.settle();
    let shot = h.frame(W as u16, H as u16, 1.0);
    let boxes = quote_boxes(&mut h);
    let sample = |boxes: &[(String, Vec<Rect>)], cfi: &str| {
        let r = boxes.iter().find(|(c, _)| c == cfi).map(|(_, rects)| rects[0]).expect("цитата не нарисована");
        (r.x0 + 3.0, r.y0 + 2.0)
    };
    let (ax, ay) = sample(&boxes, &found[0].cfi);
    let (bx, by) = sample(&boxes, &found[1].cfi);
    let (plain, active) = (distance(&shot, ax, ay), distance(&shot, bx, by));
    assert!(active > plain, "цитата, на которую перешли, не ярче остальных: {active} против {plain}");
    let inside = boxes[0].1[1].center();
    h.pointer_move(inside.x, inside.y);
    let hovered = distance(&h.frame(W as u16, H as u16, 1.0), ax, ay);
    assert!(hovered > plain, "при наведении цитата не стала насыщеннее: {hovered} против {plain}");
    assert!(hovered < active, "наведённая цитата ярче той, на которую перешли: {hovered} против {active}");
}


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
