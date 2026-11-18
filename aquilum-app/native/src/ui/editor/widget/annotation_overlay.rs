//! LANG-03c: paint-only manual annotation ranges for the active editor.
//!
//! Trusted host must validate note identity, source revision and annotation
//! digests before presenting a range. It is NOT a read grant or a Markdown edit.
//! Every source mutation and document reset clears the rendered layer.

use std::ops::Range;

use masonry::peniko::Color;
use super::TextEditor;

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
    /// Set the entire visual layer, never a partial subset after a bad span.
    /// All edits, undo/redo, external changes and note resets clear it again.
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
}

#[cfg(test)]
mod tests {
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
    fn rejects_invalid_or_unbounded_projection_as_a_whole() {
        let text = "abc abc";
        assert!(!valid_annotation_overlay(text, &[span(0..0)]));
        assert!(!valid_annotation_overlay(text, &[span(4..3)]));
        assert!(!valid_annotation_overlay(text, &[span(0..2), span(500..600)]));
        let huge = vec![span(0..3); MAX_VISUAL_ANNOTATIONS + 1];
        assert!(!valid_annotation_overlay(text, &huge));
    }

    #[test]
    fn edits_and_undo_clear_visual_annotations_before_wrong_word_paint() {
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
