use std::ops::{Range, RangeInclusive};
use std::time::Instant;

use masonry::imaging::Painter;
use masonry::peniko::Color;

use aquilum_core::text::matcher::Matcher;

use super::TextEditor;
use super::matching::{HOVER, SELECTION};
use crate::ui::theme;

/// A paint-only projection supplied by a host that has independently
/// verified note identity, full editor revision, and annotation digest.
/// This type carries neither Markdown nor filesystem authority.
#[derive(Clone, Debug)]
pub struct AnnotationPaintSpan {
    pub bytes: Range<usize>,
    pub color: Color,
}

const MAX_VISUAL_ANNOTATIONS: usize = 512;

fn valid_annotation_overlay(text: &str, spans: &[AnnotationPaintSpan]) -> bool {
    spans.len() <= MAX_VISUAL_ANNOTATIONS && spans.iter().all(|span| {
        let range = &span.bytes;
        range.start < range.end && range.end <= text.len()
            && range.end - range.start <= aquilum_core::text::annotations::MAX_ANNOTATED_SPAN_BYTES
            && text.is_char_boundary(range.start)
            && text.is_char_boundary(range.end)
            && !text[range.clone()].trim().is_empty()
    })
}

impl TextEditor {
    /// Replace the entire visual layer. Any invalid or oversized host input
    /// clears the layer rather than painting stale or partial attribution.
    /// Edits, undo/redo, external updates and document resets clear it too.
    pub fn set_annotation_overlay(
        this: &mut masonry::core::WidgetMut<'_, Self>,
        spans: Vec<AnnotationPaintSpan>,
    ) -> bool {
        let valid = if this.widget.doc.len() <= aquilum_core::text::annotations::MAX_DOCUMENT_BYTES
        {
            valid_annotation_overlay(&this.widget.doc.whole(), &spans)
        } else {
            false
        };
        this.widget.annotation_overlay = if valid { spans } else { Vec::new() };
        this.ctx.request_paint_only();
        valid
    }

    pub(super) fn shown_bytes(&self, shown: &RangeInclusive<usize>) -> Range<usize> {
        self.doc.paragraph(*shown.start()).start..self.doc.paragraph(*shown.end()).end().min(self.doc.end())
    }

    fn paint_visible_matches(&self, painter: &mut Painter<'_>, shown: RangeInclusive<usize>, matcher: &Matcher, skip: Option<&Range<usize>>, color: Color) {
        let visible = self.shown_bytes(&shown);
        matcher.each(self.doc.slice(visible.clone()), |found| {
            let occurrence = visible.start + found.start..visible.start + found.end;
            if skip != Some(&occurrence) {
                self.paint_range(painter, shown.clone(), occurrence, color);
            }
            true
        });
    }

    pub(super) fn paint_selection_matches(&self, painter: &mut Painter<'_>, shown: RangeInclusive<usize>, range: &Range<usize>) {
        if !self.smart.matches || range.len() < 2 || range.len() > 64 {
            return;
        }
        let selected = &self.doc.copy(range.clone());
        if selected.contains('\n') || selected.trim().is_empty() {
            return;
        }
        let color = theme::current().search_match_bg.multiply_alpha(0.35);
        self.paint_visible_matches(painter, shown, &Matcher::new(selected, SELECTION), Some(range), color);
    }

    pub(super) fn paint_hover_terms(&self, painter: &mut Painter<'_>, shown: RangeInclusive<usize>) {
        let color = theme::current().search_match_bg.multiply_alpha(0.5);
        self.paint_visible_matches(painter, shown, &Matcher::any(&self.hover_terms, HOVER), None, color);
    }

    pub(super) fn paint_external_reveal(&self, painter: &mut Painter<'_>, shown: RangeInclusive<usize>) {
        let Some((range, start)) = &self.external_reveal else { return };
        let elapsed = start.elapsed().as_secs_f32();
        if elapsed < 0.9 {
            let alpha = (1.0 - elapsed / 0.9).clamp(0.0, 1.0);
            let tm = theme::current();
            let color = tm.bg_accent.multiply_alpha(alpha * 0.4);
            self.paint_range(painter, shown, range.clone(), color);
        }
    }

    pub fn set_hover_terms(this: &mut masonry::core::WidgetMut<'_, Self>, terms: Vec<String>) {
        if this.widget.hover_terms != terms {
            this.widget.hover_terms = terms;
            this.ctx.request_paint_only();
        }
    }

    pub fn reveal_external_insert(this: &mut masonry::core::WidgetMut<'_, Self>, range: Range<usize>) {
        if !crate::ui::motion::on() {
            return;
        }
        this.widget.external_reveal = Some((range, Instant::now()));
        this.ctx.request_anim_frame();
        this.ctx.request_paint_only();
    }
}

#[cfg(test)]
mod annotation_tests {
    use super::*;
    use super::super::Kind;

    fn span(range: Range<usize>) -> AnnotationPaintSpan {
        AnnotationPaintSpan { bytes: range, color: Color::from_rgba8(255, 200, 0, 255) }
    }

    #[test]
    fn accepts_unicode_morpheme_and_phrases_but_not_broken_utf8() {
        let text = "Префикс и a piece of";
        let word = text.find("реф").unwrap();
        let phrase = text.find("a piece of").unwrap();
        assert!(valid_annotation_overlay(text, &[
            span(word..word + "реф".len()),
            span(phrase..phrase + "a piece of".len()),
        ]));
        assert!(!valid_annotation_overlay(text, &[span(word + 1..word + 2)]));
        assert!(!valid_annotation_overlay(text, &[span(1..2)]));
        assert!(!valid_annotation_overlay(text, &[span(0..10000)]));
    }

    #[test]
    fn rejects_empty_invalid_or_unbounded_projection_as_a_whole() {
        let text = "abc abc";
        assert!(!valid_annotation_overlay(text, &[span(0..0)]));
        assert!(!valid_annotation_overlay(text, &[span(4..3)]));
        assert!(!valid_annotation_overlay(text, &[span(0..2), span(500..600)]));
        let huge = vec![span(0..3); MAX_VISUAL_ANNOTATIONS + 1];
        assert!(!valid_annotation_overlay(text, &huge));
    }

    #[test]
    fn all_edits_and_undo_clear_visual_annotations_to_prevent_wrong_word_highlight() {
        let mut editor = TextEditor::new("run run");
        editor.annotation_overlay.push(span(0..3));
        editor.replace(0..3, "ran", Kind::Other);
        assert!(editor.annotation_overlay.is_empty());
        editor.annotation_overlay.push(span(4..7));
        editor.undo(false);
        assert!(editor.annotation_overlay.is_empty());

        editor.annotation_overlay.push(span(0..3));
        editor.apply_change(0..0, "x ".to_owned());
        assert!(editor.annotation_overlay.is_empty());
    }
}
