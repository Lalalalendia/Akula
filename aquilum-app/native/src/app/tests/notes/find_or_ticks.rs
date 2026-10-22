use super::*;

#[test]
fn page_search_or_produces_scroll_ticks_and_clears_them_on_close() {
    let mut h = Harness::new();
    let content = (0..80).map(|i| match i {
        3 => "альфа\n\n",
        27 => "бета\n\n",
        58 => "АЛЬФА и бета\n\n",
        _ => "обычная строка\n\n",
    }).collect::<String>();
    let note = h.write_note("search-or.md", &content);
    h.app.open(&mut h.root, note);
    let _ = h.root.redraw();
    h.app.open_find(&mut h.root);
    h.type_text("альфа || бета");

    let counter = |h: &Harness| h.app.find.as_ref().unwrap().counter.get(&h.root).unwrap().inner().text().to_owned();
    assert!(wait_until(80, 10, || {
        h.step();
        counter(&h) == "1/4"
    }), "поиск альтернатив не нашёл четыре совпадения");

    h.app.tick(&mut h.root, std::time::Instant::now());
    let _ = h.root.redraw();
    let scroll = h.app.note_ids.as_ref().unwrap().scroll;
    let marks = scroll.get(&h.root).unwrap().inner().find_marker_fractions().to_vec();
    assert_eq!(marks.len(), 4, "на полосе должно быть четыре отметки: {marks:?}");
    assert!(marks.iter().all(|x| x.is_finite() && (0.0..=1.0).contains(x)));
    assert!(marks.windows(2).all(|pair| pair[0] <= pair[1]));

    assert!(h.app.find_key(&mut h.root, &Key::Named(NamedKey::Escape), Modifiers::empty()));
    h.app.tick(&mut h.root, std::time::Instant::now());
    assert!(scroll.get(&h.root).unwrap().inner().find_marker_fractions().is_empty());
}
