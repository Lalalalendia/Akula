use super::*;
use crate::ui::reader::ReaderView;
use masonry::core::{PointerEvent, PointerScrollEvent, PointerState, ScrollDelta};
use masonry::dpi::PhysicalPosition;

fn reader_view<T>(h: &Harness, apply: impl FnOnce(&ReaderView) -> T) -> T {
    let Some(Popup::Reader(state)) = &h.app.popup else { panic!("expected EPUB Reader"); };
    let widget = h.root.get_widget(state.layer).expect("Reader layer");
    let reader = widget.downcast::<ReaderView>().expect("ReaderView");
    apply(reader.inner())
}

fn open_contents(h: &mut Harness) {
    let id = match &h.app.popup {
        Some(Popup::Reader(state)) => state.ids.contents.expect("EPUB must have TOC button"),
        _ => panic!("Reader is closed"),
    };
    let area = h.root.get_widget(id).unwrap().ctx().bounding_box();
    h.click(area.center().x, area.center().y);
    assert!(reader_view(h, |v| v.test_toc_bounds()).is_some(), "real pointer click did not open TOC");
}

fn click_row(h: &mut Harness, index: usize) {
    let row = reader_view(h, |v| v.test_toc_row(index)).expect("TOC row must be visible");
    h.click(row.center().x, row.center().y);
}

fn epub3_with_48_anchors(path: &std::path::Path) {
    let container = r#"<container><rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles></container>"#;
    let package = r#"<package version="3.0"><manifest><item id="nav" href="nav.xhtml" properties="nav" media-type="application/xhtml+xml"/><item id="ch" href="chapter.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="ch"/></spine></package>"#;
    let links = (0..48).map(|i| format!("<li><a href=\"chapter.xhtml#p{i}\">Фрагмент {i}</a></li>")).collect::<String>();
    let nav = format!("<html><body><nav role=\"doc-toc\"><ol>{links}</ol></nav></body></html>");
    let paragraphs = (0..48).map(|i| format!("<p id=\"p{i}\">Фрагмент {i}. {}</p>", "Synthetic long text ".repeat(24))).collect::<String>();
    let chapter = format!("<html><body><h1>Test chapter</h1>{paragraphs}</body></html>");
    crate::reader::session::epub_acceptance::write_zip(path, &[
        ("mimetype".into(), b"application/epub+zip".to_vec()),
        ("META-INF/container.xml".into(), container.as_bytes().to_vec()),
        ("OPS/book.opf".into(), package.as_bytes().to_vec()),
        ("OPS/nav.xhtml".into(), nav.into_bytes()),
        ("OPS/chapter.xhtml".into(), chapter.into_bytes()),
    ]);
}

#[test]
fn epub2_contents_click_escape_and_restart_keep_original_bytes() {
    let mut h = Harness::new();
    let (_tmp, source) = crate::reader::session::epub_acceptance::synthetic_epub();
    let files = h.vault.path().join("Files");
    std::fs::create_dir_all(&files).unwrap();
    let path = files.join("epub2.epub");
    std::fs::copy(&source, &path).unwrap();
    let original = std::fs::read(&path).unwrap();
    open_book(&mut h, &path, None, "EPUB2");
    open_contents(&mut h);
    assert!(reader_view(&h, |v| v.test_toc_row(2)).is_some());
    h.escape();
    assert!(h.reader(), "first Escape must close contents only");
    assert!(reader_view(&h, |v| v.test_toc_bounds()).is_none());
    open_contents(&mut h);
    click_row(&mut h, 2);
    assert_eq!(reader_view(&h, |v| v.test_reader_position()).unwrap().0, 2);
    let saved = reader_view(&h, |v| v.test_reader_position()).unwrap().1;
    h.escape();
    assert!(!h.reader());
    let mut h = h.restarted(1.0);
    open_book(&mut h, &path, None, "EPUB2");
    assert_eq!(reader_view(&h, |v| v.test_reader_position()).unwrap().0, 2);
    assert_eq!(reader_view(&h, |v| v.test_reader_position()).unwrap().1, saved);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    h.escape();
}

#[test]
fn epub3_contents_scroll_48_items_without_turning_page_then_jump_to_fragment() {
    let mut h = Harness::new();
    let files = h.vault.path().join("Files");
    std::fs::create_dir_all(&files).unwrap();
    let path = files.join("epub3.epub");
    epub3_with_48_anchors(&path);
    let original = std::fs::read(&path).unwrap();
    open_book(&mut h, &path, None, "EPUB3");
    let initial = reader_view(&h, |v| v.test_reader_position()).unwrap();
    open_contents(&mut h);
    let bounds = reader_view(&h, |v| v.test_toc_bounds()).unwrap();
    assert!(bounds.x0 >= 0.0 && bounds.y1 <= f64::from(H), "panel escapes viewport");
    let point = bounds.center();
    let state = PointerState { position: PhysicalPosition::new(point.x, point.y), count: 1, scale_factor: 1.0, ..PointerState::default() };
    h.root.handle_pointer_event(PointerEvent::Scroll(PointerScrollEvent {
        pointer: MOUSE, delta: ScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -100_000.0)), state,
    }));
    h.deliver();
    assert!(reader_view(&h, |v| v.test_toc_scroll()).unwrap() > 0.0);
    assert_eq!(reader_view(&h, |v| v.test_reader_position()).unwrap(), initial, "TOC scroll turned a book page");
    assert!(reader_view(&h, |v| v.test_toc_row(47)).is_some(), "last TOC row not visible");
    if let Ok(out) = std::env::var("AQ_OUT") {
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(std::path::Path::new(&out).join("reader-epub3-toc.png"), h.frame(W as u16, H as u16, 1.0).png()).unwrap();
    }
    click_row(&mut h, 47);
    let selected = reader_view(&h, |v| v.test_reader_position()).unwrap();
    assert_ne!(initial.1, selected.1, "fragment click did not move CFI");
    assert!(reader_view(&h, |v| v.test_toc_bounds()).is_none());
    let workspace = h.app.workspace.core.ui_state.resolve_workspace(&h.vault.path().to_string_lossy(), crate::dates::now_ms()).unwrap();
    let row = h.app.workspace.core.ui_state.load_reader_state(workspace, "Files/epub3.epub").unwrap().unwrap();
    assert_eq!(row.cfi.as_deref(), Some(selected.1.as_str()));
    h.escape();
    let mut h = h.restarted(1.0);
    open_book(&mut h, &path, None, "EPUB3");
    assert_eq!(reader_view(&h, |v| v.test_reader_position()).unwrap().1, selected.1);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    h.escape();
}

#[test]
fn fb2_reader_has_no_empty_epub_contents_control() {
    let mut h = Harness::new();
    let (file, page) = book_note(&h, "");
    open_book(&mut h, &file, Some(&page), "FB2");
    assert!(matches!(&h.app.popup, Some(Popup::Reader(state)) if state.ids.contents.is_none()));
    h.escape();
}

#[test]
fn toc_header_toggle_settings_switch_and_outside_dismiss_with_real_clicks() {
    let mut h = Harness::new();
    let (_tmp, source) = crate::reader::session::epub_acceptance::synthetic_epub();
    let files = h.vault.path().join("Files");
    std::fs::create_dir_all(&files).unwrap();
    let path = files.join("toggle.epub");
    std::fs::copy(&source, &path).unwrap();
    open_book(&mut h, &path, None, "EPUB");
    open_contents(&mut h);
    let toc_id = match &h.app.popup {
        Some(Popup::Reader(state)) => state.ids.contents.unwrap(),
        _ => panic!("Reader closed"),
    };
    let area = h.root.get_widget(toc_id).unwrap().ctx().bounding_box();
    h.click(area.center().x, area.center().y);
    assert!(reader_view(&h, |v| v.test_toc_bounds()).is_none(), "second click did not close TOC");

    open_contents(&mut h);
    let settings_id = match &h.app.popup {
        Some(Popup::Reader(state)) => state.ids.settings,
        _ => panic!("Reader closed"),
    };
    let settings = h.root.get_widget(settings_id).unwrap().ctx().bounding_box();
    h.click(settings.center().x, settings.center().y);
    assert!(reader_view(&h, |v| v.has_popover()), "settings did not open in one click");
    assert!(reader_view(&h, |v| v.test_toc_bounds()).is_none(), "settings and TOC overlap");
    h.escape();
    assert!(h.reader(), "Escape should dismiss settings first");

    open_contents(&mut h);
    h.click(200.0, 300.0);
    assert!(reader_view(&h, |v| v.test_toc_bounds()).is_none(), "outside click did not close TOC");
    assert!(h.reader(), "outside click closed Reader");
    h.escape();
}
