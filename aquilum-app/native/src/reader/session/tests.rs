use masonry::kurbo::Size;
use masonry::parley::{FontContext, LayoutContext};

use super::{Session, Spot, Target};
use crate::reader::tests::{book_file, sample_fb2, settings};

const VIEW: Size = Size::new(1200.0, 800.0);

fn open() -> (Session, tempfile::TempDir) {
    let (dir, file) = book_file(sample_fb2());
    (Session::open(&file, Target::Start).unwrap(), dir)
}

fn assert_only_screen(session: &Session) {
    assert!(!session.needs_layout(), "на экране абзац без раскладки");
    let shown = session.on_screen();
    assert!(!shown.is_empty());
    assert!(session.layouts.iter().all(|(item, _)| shown.contains(item)), "держится раскладка абзаца вне экрана");
}

#[test]
fn only_the_page_on_screen_keeps_its_layout() {
    let (mut fcx, mut lcx) = (FontContext::new(), LayoutContext::new());
    for scrolled in [false, true] {
        let (mut session, _dir) = open();
        for _ in 0..30 {
            session.ensure(VIEW, &settings(18.0, scrolled), &mut fcx, &mut lcx);
            assert_only_screen(&session);
            if scrolled {
                session.scroll_by(500.0);
            } else {
                session.next();
            }
        }
    }
}

#[test]
fn a_new_font_size_keeps_the_reading_place() {
    let (mut fcx, mut lcx) = (FontContext::new(), LayoutContext::new());
    let (mut session, _dir) = open();
    session.ensure(VIEW, &settings(18.0, false), &mut fcx, &mut lcx);
    for _ in 0..12 {
        session.next();
        session.ensure(VIEW, &settings(18.0, false), &mut fcx, &mut lcx);
    }
    let (block, _) = session.start().unwrap();
    for size in [26.0, 14.0] {
        session.ensure(VIEW, &settings(size, false), &mut fcx, &mut lcx);
        assert_only_screen(&session);
        let (first, last) = (session.start().unwrap().0, session.end().unwrap().0);
        assert!(first <= block && block <= last, "после смены шрифта страница {first}..{last} без абзаца {block}");
    }
}

#[test]
fn a_selection_over_both_columns_and_beyond_the_screen_makes_a_whole_quote() {
    let (mut fcx, mut lcx) = (FontContext::new(), LayoutContext::new());
    let (mut session, _dir) = open();
    session.ensure(VIEW, &settings(18.0, false), &mut fcx, &mut lcx);
    let shown = session.on_screen();
    let (from, to) = (Spot { block: shown[1], byte: 0 }, Spot { block: *shown.last().unwrap(), byte: 10 });
    let rects = session.rects(from, to);
    assert!(rects.iter().any(|r| r.x0 < VIEW.width / 2.0) && rects.iter().any(|r| r.x0 > VIEW.width / 2.0), "выделение не в обеих колонках");
    let far = Spot { block: 300, byte: "Абзац номер".len() };
    let text = session.text(from, far);
    assert!(text.starts_with("Абзац номер 1.") && text.ends_with("Абзац номер"), "{}", &text[text.len() - 40..]);
    assert!(session.range_cfi(from, far).is_some_and(|cfi| cfi.starts_with("epubcfi(/6/2!")));
}


fn reference_needs_layout(session: &Session) -> bool {
    use crate::reader::flow::Block;
    // Freeze the behavior of the old on_screen() allocation-heavy path.
    session.laid.is_none()
        || session.on_screen().into_iter().any(|item| match &session.blocks[item] {
            Block::Text { .. } => !session.layouts.iter().any(|(shown, _)| *shown == item),
            Block::Image { src, .. } => !session.images.contains_key(src),
            Block::Rule => false,
        })
}

#[test]
fn needs_layout_matches_legacy_visibility_scan_while_navigating() {
    let (mut fcx, mut lcx) = (FontContext::new(), LayoutContext::new());
    for scrolled in [false, true] {
        let (mut session, _dir) = open();
        assert!(session.needs_layout());
        assert_eq!(session.needs_layout(), reference_needs_layout(&session));
        for turn in 0..28 {
            assert_eq!(
                session.needs_layout(), reference_needs_layout(&session),
                "before ensuring: scrolled={scrolled}, turn={turn}"
            );
            session.ensure(VIEW, &settings(18.0, scrolled), &mut fcx, &mut lcx);
            assert_eq!(
                session.needs_layout(), reference_needs_layout(&session),
                "after ensuring: scrolled={scrolled}, turn={turn}"
            );
            if turn % 5 == 0 {
                session.layouts.clear();
                assert_eq!(
                    session.needs_layout(), reference_needs_layout(&session),
                    "after eviction: scrolled={scrolled}, turn={turn}"
                );
                assert!(session.needs_layout(), "evicted visible text must be refreshed");
                session.ensure(VIEW, &settings(18.0, scrolled), &mut fcx, &mut lcx);
                assert_eq!(session.needs_layout(), reference_needs_layout(&session));
            }
            if scrolled {
                session.scroll_by(447.0);
            } else {
                session.next();
            }
        }
    }
}

#[test]
#[ignore = "opt-in same-process A/B reader layout microbenchmark"]
fn needs_layout_reference_vs_fast_microbenchmark() {
    use std::hint::black_box;
    use std::time::Instant;

    fn ns_per_call(count: usize, f: impl Fn() -> bool) -> f64 {
        let started = Instant::now();
        for _ in 0..count {
            black_box(f());
        }
        started.elapsed().as_secs_f64() * 1_000_000_000.0 / count as f64
    }

    let (mut fcx, mut lcx) = (FontContext::new(), LayoutContext::new());
    for scrolled in [false, true] {
        let (mut session, _dir) = open();
        session.ensure(VIEW, &settings(18.0, scrolled), &mut fcx, &mut lcx);
        assert_eq!(reference_needs_layout(&session), session.needs_layout());
        let calls = 150_000usize;
        let mut baseline = Vec::new();
        let mut optimized = Vec::new();
        // ABBA in each trial reduces systematic scheduling and warmup bias.
        for _ in 0..7 {
            let session = black_box(&session);
            baseline.push(ns_per_call(calls, || reference_needs_layout(session)));
            optimized.push(ns_per_call(calls, || session.needs_layout()));
            optimized.push(ns_per_call(calls, || session.needs_layout()));
            baseline.push(ns_per_call(calls, || reference_needs_layout(session)));
        }
        baseline.sort_by(f64::total_cmp);
        optimized.sort_by(f64::total_cmp);
        let (before, after) = (baseline[baseline.len() / 2], optimized[optimized.len() / 2]);
        println!(
            "BENCH-NEEDS-LAYOUT mode={} baseline_ns={before:.3} optimized_ns={after:.3} change_pct={:.1}",
            if scrolled { "scroll" } else { "pages" },
            (after / before - 1.0) * 100.0
        );
    }
}


#[test]
#[ignore = "opt-in synthetic EPUB Session::ensure / first-visible-layout benchmark"]
fn session_synthetic_first_page_profile() {
    use std::hint::black_box;
    use std::path::Path;
    use std::time::Instant;

    fn milliseconds(start: Instant) -> f64 {
        start.elapsed().as_secs_f64() * 1000.0
    }
    fn middle(samples: &mut [f64]) -> f64 {
        samples.sort_by(f64::total_cmp);
        samples[samples.len() / 2]
    }

    let source = std::env::var("AQ_EPUB_SHAPE_DIR").expect("set AQ_EPUB_SHAPE_DIR to synthetic EPUB fixtures");
    for mode in ["single-br", "single-p", "split-br"] {
        let path = Path::new(&source).join(format!("{mode}.epub"));
        for scrolled in [false, true] {
            let initial = settings(18.0, scrolled);
            let changed = settings(19.5, scrolled);
            let (mut fcx, mut lcx) = (FontContext::new(), LayoutContext::new());
            let mut open_times = Vec::new();
            let mut first_times = Vec::new();
            let mut reuse_times = Vec::new();
            let mut show_times = Vec::new();
            let mut next_times = Vec::new();
            let mut reflow_times = Vec::new();
            let mut first_layouts = 0usize;
            let mut first_visible = 0usize;
            for _ in 0..5 {
                let started = Instant::now();
                let mut session = Session::open(&path, Target::Start).expect("open synthetic EPUB");
                open_times.push(milliseconds(started));
                assert_eq!(session.book.sections.len(), if mode == "split-br" { 18 } else { 1 });

                let started = Instant::now();
                session.ensure(VIEW, &initial, &mut fcx, &mut lcx);
                first_times.push(milliseconds(started));
                assert!(!session.needs_layout());
                let old_visible = session.on_screen();
                assert!(!old_visible.is_empty());
                first_layouts = session.layouts.len();
                first_visible = old_visible.len();
                assert!(first_layouts > 0, "expected retained on-screen Parley layouts");

                let started = Instant::now();
                session.ensure(VIEW, &initial, &mut fcx, &mut lcx);
                reuse_times.push(milliseconds(started));

                // Isolate the double-layout cost: the first ensure already
                // discarded its temporary full-section Parley layouts and
                // retained only the visible layouts. Evict those and rebuild
                // without recomputing the full section geometry.
                session.layouts.clear();
                assert!(session.needs_layout());
                let started = Instant::now();
                session.ensure(VIEW, &initial, &mut fcx, &mut lcx);
                show_times.push(milliseconds(started));
                assert_eq!(session.on_screen(), old_visible);
                assert!(!session.needs_layout());

                if scrolled {
                    session.scroll_by(400.0);
                } else {
                    session.next();
                }
                let started = Instant::now();
                session.ensure(VIEW, &initial, &mut fcx, &mut lcx);
                next_times.push(milliseconds(started));
                assert!(!session.needs_layout());

                let started = Instant::now();
                session.ensure(VIEW, &changed, &mut fcx, &mut lcx);
                reflow_times.push(milliseconds(started));
                assert!(!session.needs_layout());
                black_box(session.pages());
            }

            println!(
                "READER_SESSION_RAW mode={mode} scrolled={scrolled} open_ms={open_times:?} first_ensure_ms={first_times:?} reuse_ms={reuse_times:?} show_only_ms={show_times:?} next_ms={next_times:?} reflow_ms={reflow_times:?}",
            );
            println!(
                "READER_SESSION_RESULT mode={mode} scrolled={scrolled} retained_layouts={first_layouts} visible_entries={first_visible} open_ms={:.3} first_ensure_ms={:.3} reuse_ms={:.3} show_only_ms={:.3} next_ms={:.3} reflow_ms={:.3}",
                middle(&mut open_times),
                middle(&mut first_times),
                middle(&mut reuse_times),
                middle(&mut show_times),
                middle(&mut next_times),
                middle(&mut reflow_times),
            );
        }
    }
}


fn assert_same_page_state(old: &Session, new: &Session) {
    use crate::reader::page::Item;
    assert_eq!(old.pages(), new.pages(), "page count");
    assert_eq!(old.start(), new.start(), "page anchor start");
    assert_eq!(old.end(), new.end(), "page anchor end");
    let a = old.laid.as_ref().expect("baseline has paginated");
    let b = new.laid.as_ref().expect("reused version has paginated");
    assert_eq!(a.items.len(), b.items.len());
    assert_eq!(a.columns.len(), b.columns.len());
    for (index, (x, y)) in a.items.iter().zip(&b.items).enumerate() {
        match (x, y) {
            (Item::Text { lines: al, breaks: ab, x: ax, before: ap, after: aq },
             Item::Text { lines: bl, breaks: bb, x: bx, before: bp, after: bq }) => {
                assert_eq!((ab, ax.to_bits(), ap.to_bits(), aq.to_bits()),
                           (bb, bx.to_bits(), bp.to_bits(), bq.to_bits()), "text item {index}");
                assert_eq!(al.len(), bl.len(), "line count {index}");
                for (a, b) in al.iter().zip(bl) {
                    assert_eq!(
                        (a.start, a.end, a.top.to_bits(), a.bottom.to_bits()),
                        (b.start, b.end, b.top.to_bits(), b.bottom.to_bits()),
                        "line geometry {index}",
                    );
                }
            }
            (Item::Image { size: a, before: ap, after: aq },
             Item::Image { size: b, before: bp, after: bq }) => {
                assert_eq!(
                    (a.width.to_bits(), a.height.to_bits(), ap.to_bits(), aq.to_bits()),
                    (b.width.to_bits(), b.height.to_bits(), bp.to_bits(), bq.to_bits()),
                );
            }
            (Item::Rule, Item::Rule) => {}
            _ => panic!("item kind mismatch {index}"),
        }
    }
    for (a, b) in a.columns.iter().zip(&b.columns) {
        assert_eq!(a.pieces.len(), b.pieces.len());
        for (x, y) in a.pieces.iter().zip(&b.pieces) {
            assert_eq!((x.item, x.from.to_bits(), x.to.to_bits(), x.y.to_bits()),
                       (y.item, y.from.to_bits(), y.to.to_bits(), y.y.to_bits()));
        }
    }
    assert_eq!(old.on_screen(), new.on_screen(), "visible item order");
    for item in old.on_screen() {
        match (old.layout(item), new.layout(item)) {
            (Some(a), Some(b)) => {
                let a: Vec<_> = a.lines().map(|l| {
                    let m = l.metrics();
                    (l.text_range(), m.block_min_coord.to_bits(), m.block_max_coord.to_bits())
                }).collect();
                let b: Vec<_> = b.lines().map(|l| {
                    let m = l.metrics();
                    (l.text_range(), m.block_min_coord.to_bits(), m.block_max_coord.to_bits())
                }).collect();
                assert_eq!(a, b, "visible Parley layout {item}");
            }
            (None, None) => {}
            _ => panic!("visible layout presence changed for {item}"),
        }
    }
    if let (Some((item, byte)), Some((end_item, end_byte))) = (old.start(), old.end()) {
        let first = Spot { block: item, byte };
        let last = Spot { block: end_item, byte: end_byte };
        assert_eq!(old.range_cfi(first, last), new.range_cfi(first, last), "CFI");
    }
}

#[test]
fn bounded_first_page_reuse_matches_legacy_reader() {
    let large = "Синтетическая глава с длинным абзацем и переносами слов. ".repeat(1800);
    let rest: String = (0..28).map(|i| format!("<p>Проверочный абзац номер {i}.</p>")).collect();
    let source = format!(
        "<FictionBook><body><section><title><p>Глава</p></title><p>{large}</p>{rest}</section></body></FictionBook>"
    );
    let (_dir, file) = book_file(source);
    for scrolled in [false, true] {
        let mut old = Session::open(&file, Target::Start).unwrap();
        let mut new = Session::open(&file, Target::Start).unwrap();
        assert!(new.blocks.iter().any(
            |block| matches!(block, crate::reader::flow::Block::Text { text, .. } if text.len() > 16 * 1024)
        ));
        let (mut af, mut al) = (FontContext::new(), LayoutContext::new());
        let (mut bf, mut bl) = (FontContext::new(), LayoutContext::new());
        old.ensure_without_reuse(VIEW, &settings(18.0, scrolled), &mut af, &mut al);
        new.ensure(VIEW, &settings(18.0, scrolled), &mut bf, &mut bl);
        assert_same_page_state(&old, &new);
        assert!(!new.needs_layout());
        let reused = new.on_screen().into_iter().filter(
            |&item| new.layout(item).is_some()
        ).count();
        assert!(reused > 0);
        if scrolled {
            old.scroll_by(380.0);
            new.scroll_by(380.0);
        } else {
            assert_eq!(old.next(), new.next());
        }
        old.ensure_without_reuse(VIEW, &settings(18.0, scrolled), &mut af, &mut al);
        new.ensure(VIEW, &settings(18.0, scrolled), &mut bf, &mut bl);
        assert_same_page_state(&old, &new);
        old.ensure_without_reuse(VIEW, &settings(21.0, scrolled), &mut af, &mut al);
        new.ensure(VIEW, &settings(21.0, scrolled), &mut bf, &mut bl);
        assert_same_page_state(&old, &new);
    }
}

#[test]
#[ignore = "paired release ABBA of first ensure (synthetic, not desktop first paint)"]
fn initial_layout_reuse_abba_profile() {
    use std::hint::black_box;
    use std::path::Path;
    use std::time::Instant;

    fn median(mut samples: Vec<f64>) -> f64 {
        samples.sort_by(f64::total_cmp);
        (samples[6] + samples[7]) * 0.5
    }

    let folder = std::env::var("AQ_EPUB_SHAPE_DIR").expect("AQ_EPUB_SHAPE_DIR");
    let mut fcx = FontContext::new();
    let mut lcx = LayoutContext::new();
    for mode in ["single-br", "single-p", "split-br"] {
        let path = Path::new(&folder).join(format!("{mode}.epub"));
        for scrolled in [false, true] {
            let settings = settings(18.0, scrolled);
            let mut old_ms = Vec::new();
            let mut reused_ms = Vec::new();
            let mut retained = 0usize;
            let mut visible = 0usize;
            let mut benchmark = |reuse: bool, fcx: &mut FontContext, lcx: &mut LayoutContext<masonry::core::BrushIndex>| -> f64 {
                let mut session = Session::open(&path, Target::Start).unwrap();
                let start = Instant::now();
                if reuse {
                    session.ensure(VIEW, &settings, fcx, lcx);
                } else {
                    session.ensure_without_reuse(VIEW, &settings, fcx, lcx);
                }
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                assert!(!session.needs_layout());
                if reuse {
                    retained = session.layouts.len();
                    visible = session.on_screen().len();
                }
                black_box(session);
                elapsed
            };
            for _ in 0..2 {
                black_box(benchmark(false, &mut fcx, &mut lcx));
                black_box(benchmark(true, &mut fcx, &mut lcx));
            }
            let mut first = Session::open(&path, Target::Start).unwrap();
            let mut second = Session::open(&path, Target::Start).unwrap();
            first.ensure_without_reuse(VIEW, &settings, &mut fcx, &mut lcx);
            second.ensure(VIEW, &settings, &mut fcx, &mut lcx);
            assert_same_page_state(&first, &second);
            for _ in 0..7 {
                old_ms.push(benchmark(false, &mut fcx, &mut lcx));
                reused_ms.push(benchmark(true, &mut fcx, &mut lcx));
                reused_ms.push(benchmark(true, &mut fcx, &mut lcx));
                old_ms.push(benchmark(false, &mut fcx, &mut lcx));
            }
            let old_median = median(old_ms.clone());
            let new_median = median(reused_ms.clone());
            println!(
                "READER_REUSE_RAW mode={mode} scrolled={scrolled} old_ms={old_ms:?} reused_ms={reused_ms:?}",
            );
            println!(
                "READER_REUSE_RESULT mode={mode} scrolled={scrolled} visible={visible} retained={retained} old_ms={old_median:.3} reused_ms={new_median:.3} speedup={:.3}",
                old_median / new_median,
            );
        }
    }
}
