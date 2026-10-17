//! Bounded, conservative transfer of plain Parley layouts between clean tabs.
//! Snapshot candidates become usable only after the separate file-hash gate
//! and exact rendering-context verification in the new Document. Rich Markdown,
//! caret/IME, media and resolver-dependent decorations are never transferred.
use masonry::core::{BrushIndex, StyleSet};
use masonry::parley::Layout;

#[cfg(test)]
thread_local! {
    static LAST_APPLIED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn take_applied() -> usize {
    LAST_APPLIED.with(|v| v.replace(0))
}

use super::super::markdown::{Block, Decor, Fonts};
use super::{Document, Fold};

const MAX_WINDOW_BYTES: usize = 512 * 1024;
const MAX_PARAGRAPHS: usize = 32;
const MAX_TEXT_BYTES: usize = 16 * 1024;
const MAX_SINGLE_LINE_BYTES: usize = 1024;

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
    theme_generation: usize,
    editor_font: crate::ui::theme::EditorFont,
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
        // report_edits() deliberately keeps Some(Vec::new()) in normal UI
        // widgets; an empty event buffer is not an unsaved document edit.
        if !self.placed || self.dirty.is_some()
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
            theme_generation: crate::ui::theme::generation(),
            editor_font: (*crate::ui::theme::editor_font_settings()).clone(),
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
        // IME preedit text changes shaping and blocks all reuse. A normal
        // caret is different: the usual Document::measure() invalidates only
        // the caret-bearing line because snapshot decorations have caret=None.
        // Do not throw out dozens of unrelated plain paragraphs for focus.
        // Plain, non-interactive text glyph shaping is width-independent.
        // Newly inserted layouts are rebroken by the standard first-layout
        // width transition below, so we never reuse stale line geometry.
        // This also handles the initial scrollbar appearing (a transient
        // ~16px difference) without throwing away correctly shaped text.
        if compose.is_some()
            || width.is_none() || !self.windowed.eq(&snapshot.windowed)
            || self.full.byte_len() != snapshot.full_len || self.base != snapshot.base
            || self.text != snapshot.text
            || fonts_key(self.markdown.as_ref()) != snapshot.font_key
            || styles_key(styles) != snapshot.style_key
            || snapshot.theme_generation != crate::ui::theme::generation()
            || snapshot.editor_font != *crate::ui::theme::editor_font_settings()
        {
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
            #[cfg(test)]
            LAST_APPLIED.with(|v| v.set(v.get() + reused));
        }
        // Do not bypass the usual first-layout width transition: it performs
        // offscreen height re-estimation, scroll anchoring and composition.
        // Rebreaking an already shaped line at the same width is cheap, and
        // preserves the exact cold-page lifecycle.
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
        for case in [0, 2, 3, 4, 5, 6] {
            let mut snapshot = baseline.trial_snapshot(&styles).unwrap();
            let changed = if case == 0 { original.replace("обычный", "другой ") } else { original.clone() };
            let mut fresh = doc(&changed);
            if case == 2 {
                fresh.markdown.as_mut().unwrap().size = 19.0;
            }
            if case == 5 {
                snapshot.theme_generation = snapshot.theme_generation.wrapping_add(1);
            }
            if case == 6 {
                snapshot.editor_font.size += 1.0;
            }
            fresh.trial_offer(snapshot);
            let width = if case == 1 { Some(550.0) } else { Some(600.0) };
            let compose = (case == 3).then_some((0, "превью"));
            let different_style = StyleSet::new(18.0);
            let current_styles = if case == 4 { &different_style } else { &styles };
            let count = fresh.try_restore_trial(width, current_styles, compose, None);
            assert_eq!(count, 0, "invalidated case {case}");
        }
    }

    #[test]
    fn width_mismatch_rebreaks_all_cached_plain_line_geometry() {
        let text = "Много Unicode слов для переноса 😺 и ещё немного текста.\n".repeat(75);
        let styles = StyleSet::new(16.0);
        let mut before = doc(&text);
        shape(&mut before, 600.0, &styles);
        let snap = before.trial_snapshot(&styles).expect("some ordinary paragraphs shaped");
        let mut reused = doc(&text);
        reused.trial_offer(snap);
        assert!(reused.try_restore_trial(Some(361.0), &styles, None, None) > 0);
        shape(&mut reused, 361.0, &styles);

        let mut fresh = doc(&text);
        shape(&mut fresh, 361.0, &styles);
        assert!((reused.height() - fresh.height()).abs() < 0.001);
        for i in 0..fresh.lines.count() {
            assert!((reused.heights.get(i) - fresh.heights.get(i)).abs() < 0.001, "line {i} keeps stale wrapping");
        }
        assert!(reused.laid.get(0).is_some());
    }

    #[test]
    fn focused_caret_reuses_unrelated_plain_lines_and_keeps_geometry() {
        let content = "обычная Unicode строка 🐱\n".repeat(120);
        let styles = StyleSet::new(16.0);
        let mut previous = doc(&content);
        shape(&mut previous, 600.0, &styles);
        let candidate = previous.trial_snapshot(&styles).expect("plain lines shaped");

        let mut reopened = doc(&content);
        reopened.trial_offer(candidate);
        let restored = reopened.try_restore_trial(Some(600.0), &styles, None, Some(0));
        assert!(restored > 0, "focused note should still reuse non-caret paragraphs");
        reopened.layout(&mut FontContext::default(), &mut LayoutContext::new(), &styles, Some(600.0), None, Some(0));
        let mut fresh = doc(&content);
        fresh.layout(&mut FontContext::default(), &mut LayoutContext::new(), &styles, Some(600.0), None, Some(0));
        assert!((fresh.height() - reopened.height()).abs() < 0.001);
        assert!(reopened.laid.get(0).is_some_and(|laid| laid.decor.caret == Some(0)),
            "line containing the active caret must be reshaped, not blindly reused");
    }

    #[test]
    fn clean_editor_with_edit_reporting_still_captures_plain_lines() {
        let text = "строка обычного текста\n".repeat(48);
        let styles = StyleSet::new(16.0);
        let mut original = doc(&text);
        original.report_edits();
        shape(&mut original, 600.0, &styles);
        assert!(original.trial_snapshot(&styles).is_some(), "Some(empty) is not a dirty document");
        original.replace(0..0, "правка ");
        assert!(original.trial_snapshot(&styles).is_none(), "pending changes must block reuse");
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
        let text = "Короткий обычный абзац с кириллицей, emoji 🐱 и поиском.\n".repeat(700)
            + &"**выделение** и [ссылка](https://example.org)\n".repeat(12);
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
