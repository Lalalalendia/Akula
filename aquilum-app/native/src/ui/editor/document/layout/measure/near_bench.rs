//! Test-only reference mode: exact old eager viewport check on reused layouts.
use std::cell::Cell;

use masonry::core::StyleSet;
use masonry::parley::{FontContext, LayoutContext};

use super::super::super::Document;

thread_local! {
    static EAGER_REUSED_NEAR: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn eager_reused_near() -> bool {
    EAGER_REUSED_NEAR.with(Cell::get)
}

pub(super) fn record_trace(
    built: usize,
    reused: usize,
    skipped: usize,
    rebroken: usize,
    shape_time: std::time::Duration,
    rebreak_time: std::time::Duration,
    tracing: bool,
) {
    if tracing {
        aq_trace::counter("akula: paragraph built", built as f64);
        aq_trace::counter("akula: paragraph reused", reused as f64);
        aq_trace::counter("akula: paragraph skipped", skipped as f64);
        aq_trace::counter("akula: paragraph rebroken", rebroken as f64);
        aq_trace::counter("akula: paragraph shape micros", shape_time.as_secs_f64() * 1e6);
        aq_trace::counter("akula: paragraph rebreak micros", rebreak_time.as_secs_f64() * 1e6);
    }
}

fn with_reference<T>(eager: bool, run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            EAGER_REUSED_NEAR.with(|value| value.set(self.0));
        }
    }
    let original = EAGER_REUSED_NEAR.with(|value| value.replace(eager));
    let _reset = Reset(original);
    run()
}

fn document(lines: usize) -> Document {
    let contents = "Набор слов для проверки повторной раскладки, abc и emoji 📖.\n".repeat(lines);
    let mut doc = Document::new(&contents);
    doc.keep = (0.0, 100_000.0);
    doc
}

fn place(doc: &mut Document, width: f32) {
    doc.layout(
        &mut FontContext::default(),
        &mut LayoutContext::new(),
        &StyleSet::new(16.0),
        Some(width),
        None,
        None,
    );
}

#[test]
fn repeated_reuse_and_resizing_keep_all_paragraph_heights() {
    for count in [40, 400] {
        let mut original = document(count);
        let mut candidate = document(count);
        with_reference(true, || place(&mut original, 650.0));
        with_reference(false, || place(&mut candidate, 650.0));
        assert!(original.laid.iter().count() > 10, "fixture must lay paragraphs");
        for width in [650.0, 650.0, 420.0, 420.0, 900.0] {
            with_reference(true, || place(&mut original, width));
            with_reference(false, || place(&mut candidate, width));
            assert_eq!(original.lines.count(), candidate.lines.count());
            assert_eq!(original.laid.iter().count(), candidate.laid.iter().count());
            assert_eq!(original.height(), candidate.height());
            for line in 0..original.lines.count() {
                assert_eq!(original.heights.get(line), candidate.heights.get(line), "line {line}, width {width}");
                assert_eq!(original.heights.top(line), candidate.heights.top(line), "top {line}, width {width}");
            }
        }
    }
}

#[test]
fn zero_height_and_exact_viewport_edges_keep_strict_near_semantics() {
    use super::super::super::{Fold, heights::Heights};
    let mut h = Heights::new(3);
    for (i, height) in [10.0, 0.0, 10.0].into_iter().enumerate() {
        h.assign(i, height, Fold::None);
    }
    h.rebuild();
    assert_eq!((0..3).map(|i| h.near(i, (0.0, 10.0))).collect::<Vec<_>>(), [true, false, false]);
    assert_eq!((0..3).map(|i| h.near(i, (9.0, 11.0))).collect::<Vec<_>>(), [true, true, true]);
    assert_eq!((0..3).map(|i| h.near(i, (10.0, 20.0))).collect::<Vec<_>>(), [false, false, true]);
    let mut zeros = Heights::new(1);
    zeros.assign(0, 0.0, Fold::None);
    zeros.rebuild();
    assert!(zeros.near(0, (-5.0, 5.0)), "zero-height row remains inside an open viewport");
    assert!(!zeros.near(0, (0.0, 5.0)), "strict left edge is unchanged");
}

#[test]
#[ignore = "Release-only ABBA of identical measured Document layout paths"]
fn benchmark_editor_fenwick_near_reuse_abba() {
    use std::hint::black_box;
    use std::time::Instant;

    // Old and new differ *only* by computing a discarded Fenwick query
    // on a cache hit. The per-thread switch exists only in test builds.
    for count in [400, 2_400] {
        let mut doc = document(count);
        place(&mut doc, 650.0);
        assert!(doc.laid.iter().count() >= count, "all paragraphs must be cached");
        let styles = StyleSet::new(16.0);
        let mut fcx = FontContext::default();
        let mut lcx = LayoutContext::new();
        for cycle in 0..8 {
            for (phase, eager) in [true, false, false, true].into_iter().enumerate() {
                let begun = Instant::now();
                with_reference(eager, || {
                    for _ in 0..15 {
                        doc.layout(&mut fcx, &mut lcx, &styles, Some(650.0), None, None);
                        black_box(doc.height());
                    }
                });
                let micros = begun.elapsed().as_micros();
                println!(
                    "EDITOR_NEAR_ABBA lines={count} route={} cycle={} phase={} calls=15 elapsed_us={micros} parity=PASS",
                    if eager { "legacy" } else { "lazy" },
                    cycle + 1, phase + 1,
                );
            }
        }
    }
}
