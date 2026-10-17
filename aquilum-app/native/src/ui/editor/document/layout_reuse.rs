//! Test-only proof of bounded Parley layout transfer across distinct Documents.
//!
//! This is intentionally *not* enabled for normal Akula builds. Parley layout
//! state may depend on style, width, caret, resolver and live media. A durable
//! production cache needs stronger invalidation and paired UI/fidelity tests.
use masonry::core::{BrushIndex, StyleSet};
use masonry::parley::Layout;

use super::super::markdown::{Block, Decor, Fonts};
use super::{Document, Fold};

thread_local! {
    static UI_RESTORED_PARAGRAPHS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

/// Cleared between test-only tab selections; never compiled in release builds.
pub(crate) fn take_ui_restore_hits() -> usize {
    UI_RESTORED_PARAGRAPHS.with(|v| v.replace(0))
}

pub(crate) fn record_ui_restore_hits(count: usize) {
    UI_RESTORED_PARAGRAPHS.with(|v| v.set(v.get() + count));
}

// Count independently the reasons a snapshot cannot be used by the actual
// Masonry layout pass. This diagnostic is never built into release binaries.
#[derive(Clone, Copy, Default)]
pub(crate) struct RestoreGateCounts {
    pub attempted: usize,
    pub width: usize,
    pub full_len: usize,
    pub base: usize,
    pub text: usize,
    pub windowed: usize,
    pub fonts: usize,
    pub style: usize,
    pub ime: usize,
    pub caret: usize,
    pub passed: usize,
    pub deferred_initial_width: usize,
}
thread_local! {
    static RESTORE_GATES: std::cell::Cell<RestoreGateCounts> =
        std::cell::Cell::new(RestoreGateCounts::default());
}
pub(crate) fn take_restore_gate_counts() -> RestoreGateCounts {
    RESTORE_GATES.with(|v| v.replace(RestoreGateCounts::default()))
}

const MAX_WINDOW_BYTES: usize = 512 * 1024;
const MAX_PARAGRAPHS: usize = 32;
const MAX_TEXT_BYTES: usize = 128 * 1024;
const MAX_SINGLE_LINE_BYTES: usize = 16 * 1024;

struct PlainLayout {
    line: usize,
    layout: Layout<BrushIndex>,
}

pub(crate) struct Snapshot {
    base: usize,
    text: String,
    full_len: usize,
    windowed: bool,
    width: f32,
    font_key: Option<(u32, String, bool, bool)>,
    style_key: Vec<String>,
    entries: Vec<PlainLayout>,
}

fn fonts_key(fonts: Option<&Fonts>) -> Option<(u32, String, bool, bool)> {
    fonts.map(|f| (f.size.to_bits(), format!("{:?}", f.mono), f.callouts, f.tags))
}

fn styles_key(styles: &StyleSet) -> Vec<String> {
    // Strict matching matters more than hit rate in this isolated experiment.
    // Sorting makes the comparison independent of StyleSet's storage order.
    let mut parts: Vec<_> = styles.inner().values().map(|x| format!("{x:?}")).collect();
    parts.sort_unstable();
    parts
}

fn plain_decor(decor: &Decor) -> bool {
    decor.x == 0.0 && decor.inset == 0.0 && decor.pad == (0.0, 0.0)
        && decor.hang.is_none() && decor.caret.is_none()
        && decor.codes.is_empty() && decor.marks.is_empty()
        && decor.tags.is_empty() && decor.links.is_empty()
        && decor.boxes.is_empty() && decor.pictures.is_empty()
}

impl Document {
    /// Copy only plain, non-interactive paragraph layouts. Ineligible blocks
    /// are always shaped through the normal Markdown/Parley pipeline.
    pub(crate) fn trial_snapshot(&self, styles: &StyleSet) -> Option<Snapshot> {
        if !self.placed || self.dirty.is_some()
            // report_edits() sets Some(Vec::new()) for normal clean editors.
            // Reject real buffered edits, not an empty reporting queue.
            || self.edits.as_ref().is_some_and(|edits| !edits.is_empty())
            || self.editing.is_some() || self.text.len() > MAX_WINDOW_BYTES
        {
            return None;
        }
        let width = self.width?;
        let mut indices: Vec<usize> = self.laid.iter()
            .map(|(i, _)| i)
            .filter(|&i| i < self.lines.count() && *self.blocks.get(i) == Block::Text)
            .collect();
        indices.sort_unstable();
        let mut entries = Vec::new();
        let mut bytes: usize = 0;
        for i in indices {
            if entries.len() == MAX_PARAGRAPHS {
                break;
            }
            let range = self.lines.range(i);
            let size = range.len();
            if size > MAX_SINGLE_LINE_BYTES || bytes.saturating_add(size) > MAX_TEXT_BYTES {
                continue;
            }
            let Some(laid) = self.laid.get(i) else { continue };
            if !plain_decor(&laid.decor) {
                continue;
            }
            bytes += size;
            entries.push(PlainLayout { line: i, layout: laid.layout.clone() });
        }
        (!entries.is_empty()).then(|| Snapshot {
            base: self.base,
            text: self.text.clone(),
            full_len: self.full.byte_len(),
            windowed: self.windowed,
            width,
            font_key: fonts_key(self.markdown.as_ref()),
            style_key: styles_key(styles),
            entries,
        })
    }

    /// Install a candidate; no layouts are reused until the next width-aware
    /// layout call validates the same content and rendering context.
    pub(crate) fn trial_offer(&mut self, snapshot: Snapshot) {
        self.trial_snapshot = Some(snapshot);
    }

    pub(super) fn try_restore_trial(
        &mut self,
        width: Option<f32>,
        styles: &StyleSet,
        compose: Option<(usize, &str)>,
        caret: Option<usize>,
    ) -> usize {
        let Some(snapshot) = self.trial_snapshot.take() else { return 0 };
        // During Masonry's first vertical measurement, the editor may be
        // measured at a provisional width before the real layout size is
        // assigned. Defer ONLY that first-width mismatch. Never apply a layout
        // without validating its final, exact width and entire context.
        let same_content_context = self.full.byte_len() == snapshot.full_len
            && self.base == snapshot.base && self.text == snapshot.text
            && self.windowed == snapshot.windowed
            && fonts_key(self.markdown.as_ref()) == snapshot.font_key
            && styles_key(styles) == snapshot.style_key
            && compose.is_none() && caret.is_none();
        if self.width.is_none() && width != Some(snapshot.width) && same_content_context {
            RESTORE_GATES.with(|slot| {
                let mut counts = slot.get();
                counts.deferred_initial_width += 1;
                slot.set(counts);
            });
            self.trial_snapshot = Some(snapshot);
            return 0;
        }
        // IME composition and caret-dependent Markdown can change the Parley
        // shape of a line even if the file bytes remain identical.
        let blocked = RESTORE_GATES.with(|slot| {
            let mut v = slot.get();
            v.attempted += 1;
            let failed = width != Some(snapshot.width)
                || self.full.byte_len() != snapshot.full_len || self.base != snapshot.base
                || self.text != snapshot.text || self.windowed != snapshot.windowed
                || fonts_key(self.markdown.as_ref()) != snapshot.font_key
                || styles_key(styles) != snapshot.style_key || compose.is_some() || caret.is_some();
            v.width += usize::from(width != Some(snapshot.width));
            v.full_len += usize::from(self.full.byte_len() != snapshot.full_len);
            v.base += usize::from(self.base != snapshot.base);
            v.text += usize::from(self.text != snapshot.text);
            v.windowed += usize::from(self.windowed != snapshot.windowed);
            v.fonts += usize::from(fonts_key(self.markdown.as_ref()) != snapshot.font_key);
            v.style += usize::from(styles_key(styles) != snapshot.style_key);
            v.ime += usize::from(compose.is_some());
            v.caret += usize::from(caret.is_some());
            if !failed {
                v.passed += 1;
            }
            slot.set(v);
            failed
        });
        if blocked {
            return 0;
        }
        let mut reused = 0;
        for item in snapshot.entries {
            if item.line >= self.lines.count() || *self.blocks.get(item.line) != Block::Text {
                continue;
            }
            let height = f64::from(item.layout.height());
            self.heights.set_text(item.line, height);
            self.heights.set(item.line, height, Fold::None);
            self.laid.insert(item.line, super::laid::Laid { layout: item.layout, decor: Decor::default() });
            reused += 1;
        }
        if reused != 0 {
            // Preserve the validated width so that first layout won't
            // gratuitously rewrap layouts that already use that width.
            self.width = width;
        }
        reused
    }
}

#[cfg(test)]
mod tests {
    use masonry::core::StyleSet;
    use masonry::parley::{FontContext, LayoutContext};

    use super::Document;
    use crate::ui::editor::markdown::Fonts;

    fn doc(text: &str) -> Document {
        let mut d = Document::windowed(crop::Rope::from(text));
        d.set_markdown(Some(Fonts {
            size: 16.0,
            mono: crate::ui::theme::editor_font("iA Writer Mono"),
            ch: 0.0,
            callouts: true,
            tags: true,
        }));
        d.keep = (0.0, 2_000.0);
        d
    }

    fn shape(d: &mut Document, width: f32, styles: &StyleSet) {
        d.layout(&mut FontContext::default(), &mut LayoutContext::new(), styles, Some(width), None, None);
    }

    #[test]
    fn only_plain_paragraphs_can_be_reused_and_layout_geometry_stays_equal() {
        let content = "обычный текст и emoji 🐱\n".repeat(120)
            + "**жирный** и [ссылка](https://example.org)\n"
            + &"последняя строка\n".repeat(12);
        let styles = StyleSet::new(16.0);
        let mut baseline = doc(&content);
        shape(&mut baseline, 600.0, &styles);
        let snap = baseline.trial_snapshot(&styles).expect("plain cache candidates");
        assert!(!snap.entries.is_empty());
        assert!(snap.entries.len() <= super::MAX_PARAGRAPHS);
        assert!(snap.entries.iter().all(|item| item.line < 120));

        let mut restored = doc(&content);
        restored.trial_offer(snap);
        let reused = restored.try_restore_trial(Some(600.0), &styles, None, None);
        assert!(reused > 0, "test must prove a real snapshot hit, not just equal fresh output");
        shape(&mut restored, 600.0, &styles);
        let mut fresh = doc(&content);
        shape(&mut fresh, 600.0, &styles);
        assert_eq!(fresh.loaded(), restored.loaded());
        assert!((fresh.height() - restored.height()).abs() < 0.001);
        for i in 0..fresh.lines.count() {
            assert!((fresh.heights.get(i) - restored.heights.get(i)).abs() < 0.001, "line {i}");
        }
    }

    #[test]
    fn changing_bytes_width_font_or_ime_never_injects_old_layouts() {
        let original = "обычный текст\n".repeat(40);
        let styles = StyleSet::new(16.0);
        let mut baseline = doc(&original);
        shape(&mut baseline, 600.0, &styles);
        for case in 0..4 {
            let snapshot = baseline.trial_snapshot(&styles).unwrap();
            let changed = if case == 0 { original.replace("обычный", "другой ") } else { original.clone() };
            let mut fresh = doc(&changed);
            if case == 2 {
                fresh.markdown.as_mut().unwrap().size = 19.0;
            }
            fresh.trial_offer(snapshot);
            let width = if case == 1 { Some(550.0) } else { Some(600.0) };
            let compose = (case == 3).then_some((0, "превью"));
            let count = fresh.try_restore_trial(width, &styles, compose, None);
            assert_eq!(count, 0, "invalidated case {case}");
        }
    }

    #[test]
    fn clean_editor_edit_reporting_does_not_suppress_reuse() {
        let content = "обычный текст и Unicode 😀\\n".replace("\\n", "\n").repeat(48);
        let styles = StyleSet::new(16.0);
        let mut doc = doc(&content);
        doc.report_edits();
        shape(&mut doc, 600.0, &styles);
        assert!(doc.trial_snapshot(&styles).is_some(), "clean note has an empty edit-reporting queue");
        doc.report(0..0, "inserted");
        assert!(doc.trial_snapshot(&styles).is_none(), "actual unconsumed edits must disable transfer");
    }

    #[test]
    fn provisional_measure_width_defers_once_then_real_width_restores_plain_paragraphs() {
        let text = "обычный текст и русский абзац\\n".replace("\\n", "\n").repeat(80);
        let styles = StyleSet::new(16.0);
        let mut source = doc(&text);
        shape(&mut source, 600.0, &styles);
        let snapshot = source.trial_snapshot(&styles).expect("test requires layouts");
        let mut target = doc(&text);
        target.trial_offer(snapshot);
        // An initial measure at a provisional width must not consume an
        // otherwise-correct snapshot for the subsequent full-width layout.
        shape(&mut target, 350.0, &styles);
        assert!(target.trial_snapshot.is_some(), "first width mismatch is deferred");
        let applied = target.try_restore_trial(Some(600.0), &styles, None, None);
        assert!(applied > 0, "exact final wrap width can use the snapshot");
        shape(&mut target, 600.0, &styles);
        let mut fresh = doc(&text);
        shape(&mut fresh, 600.0, &styles);
        assert_eq!(fresh.loaded(), target.loaded());
        assert!((fresh.height() - target.height()).abs() < 0.001);
    }

    #[test]
    fn genuinely_different_final_width_drops_candidate() {
        let text = "длинная обычная строка для переноса\\n".replace("\\n", "\n").repeat(80);
        let styles = StyleSet::new(16.0);
        let mut source = doc(&text);
        shape(&mut source, 600.0, &styles);
        let snapshot = source.trial_snapshot(&styles).expect("expected plain layouts");
        let mut target = doc(&text);
        target.trial_offer(snapshot);
        shape(&mut target, 350.0, &styles);
        assert!(target.trial_snapshot.is_some());
        // A second layout at the *same* incompatible width is treated as
        // the actual width, consumes the pending snapshot and rejects reuse.
        shape(&mut target, 350.0, &styles);
        assert!(target.trial_snapshot.is_none(), "different final width never retains stale shapes");
        assert_eq!(target.try_restore_trial(Some(600.0), &styles, None, None), 0);
    }

    #[test]
    fn snapshot_rejects_pending_document_edits() {
        let text = "текст\n".repeat(32);
        let styles = StyleSet::new(16.0);
        let mut original = doc(&text);
        shape(&mut original, 600.0, &styles);
        original.replace(0..0, "изменено ");
        assert!(original.trial_snapshot(&styles).is_none());
    }

    #[test]
    #[ignore = "opt-in paired same-process diagnostic, not a CI speed threshold"]
    fn benchmark_parley_plain_reuse_abba() {
        use std::hint::black_box;
        use std::time::Instant;

        // Both paths use the same rendered Unicode note and a shared Parley
        // font/layout context. Include snapshot creation in the cached path.
        let text = (
            "Короткий обычный абзац с кириллицей, emoji 🐱 и поиском.\n".repeat(700)
                + &"**выделение** и [ссылка](https://example.org)\n".repeat(12)
        );
        let styles = StyleSet::new(16.0);
        let mut fcx = FontContext::default();
        let mut lcx = LayoutContext::new();
        let mut original = doc(&text);
        original.layout(&mut fcx, &mut lcx, &styles, Some(600.0), None, None);
        let baseline_snapshot = original.trial_snapshot(&styles).expect("candidate paragraphs exist");
        assert!(!baseline_snapshot.entries.is_empty());
        drop(baseline_snapshot);

        // Warm up font loading and both paths before measured samples.
        for reuse in [false, true, true, false] {
            let mut d = doc(&text);
            if reuse {
                d.trial_offer(original.trial_snapshot(&styles).unwrap());
            }
            d.layout(&mut fcx, &mut lcx, &styles, Some(600.0), None, None);
            black_box(d.height());
        }

        fn median(values: &mut [f64]) -> f64 {
            values.sort_by(|a, b| a.total_cmp(b));
            let n = values.len();
            (values[n / 2 - 1] + values[n / 2]) / 2.0
        }
        let mut baseline = Vec::new();
        let mut cached = Vec::new();
        for _ in 0..6 {
            for reuse in [false, true, true, false] {
                let start = Instant::now();
                for _ in 0..6 {
                    let mut d = doc(&text);
                    if reuse {
                        d.trial_offer(original.trial_snapshot(&styles).unwrap());
                    }
                    d.layout(&mut fcx, &mut lcx, &styles, Some(600.0), None, None);
                    black_box(d.height());
                }
                let mean_ns = start.elapsed().as_nanos() as f64 / 6.0;
                (if reuse { &mut cached } else { &mut baseline }).push(mean_ns);
            }
        }
        let normal = median(&mut baseline);
        let reused = median(&mut cached);
        println!(
            "PARLEY_REUSE_ABBA baseline_ns={normal:.1} candidate_ns={reused:.1} baseline_over_candidate={:.3} repeats={} note_bytes={}",
            normal / reused, baseline.len(), text.len()
        );
        assert!(normal.is_finite() && reused.is_finite() && normal > 0.0 && reused > 0.0);
    }
}
