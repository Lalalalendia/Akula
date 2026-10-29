use std::{borrow::Cow, ops::Range};

use masonry::core::{BrushIndex, StyleSet};
use masonry::parley::{Alignment, FontContext, Layout, LayoutContext};

use super::super::markdown::{self, Block, Decor, Fonts};
use super::super::highlight::Lang;
use super::{Document, text};

pub(super) static PLAIN: Decor = Decor { x: 0.0, inset: 0.0, pad: (0.0, 0.0), hang: None, codes: Vec::new(), marks: Vec::new(), tags: Vec::new(), links: Vec::new(), caret: None, source_selection: None, boxes: Vec::new(), pictures: Vec::new() };

#[cfg(test)]
#[path = "laid_tests.rs"]
mod tests;

// The OS preedit is visual, never a mutation of the durable document.
// Only a selection wholly inside this one paragraph is visually replaced;
// the general multiline composition layout remains a separate acceptance gate.
fn preview_preedit(source: &str, span: Range<usize>, at: usize, selection: Option<&Range<usize>>, preedit: &str) -> Option<(String, Range<usize>)> {
    let local = at.checked_sub(span.start)?;
    let end = selection
        .filter(|selected| selected.start == at && selected.end <= span.end)
        .and_then(|selected| selected.end.checked_sub(span.start))
        .unwrap_or(local);
    let prefix = source.get(..local)?;
    let suffix = source.get(end..)?;
    let mut shown = String::with_capacity(prefix.len() + preedit.len() + suffix.len());
    shown.push_str(prefix);
    shown.push_str(preedit);
    shown.push_str(suffix);
    Some((shown, local..local + preedit.len()))
}

pub(super) struct Laid {
    pub(super) layout: Layout<BrushIndex>,
    pub(super) decor: Decor,
}

impl Laid {
    pub(super) fn height(&self) -> f64 {
        f64::from(self.layout.height()) + self.decor.pad.0 + self.decor.pad.1
    }
}

impl Document {
    /// Fences and their body are unwrapped, but indented code and math retain
    /// the historical layout. Avoid backwards scans on every paragraph paint.
    pub(in crate::ui::editor) fn fenced_code(&self, line: usize) -> bool {
        match self.blocks.get(line) {
            Block::Code { lang: Some(Lang::Math), .. } => false,
            Block::Code { fence: true, .. } => true,
            Block::Code { fence: false, .. } => self.blocks.state(line).is_some_and(|state| state.has_open_fence()),
            _ => false,
        }
    }

    pub(super) fn context<'a>(&'a self, fonts: &'a Fonts, line: Option<usize>) -> markdown::Context<'a> {
        markdown::Context { fonts, unresolved: &self.unresolved.0, resolver: self.resolver.as_deref(), source_selection: line.and_then(|i| self.local_selection(i)) }
    }

    pub(super) fn local_caret(&self, line: usize, caret: Option<usize>) -> Option<usize> {
        let range = self.span(line);
        self.markdown.as_ref().and(caret).filter(|c| range.start <= *c && *c <= range.end).map(|c| c - range.start)
    }

    pub(super) fn lay(&self, line: usize, fcx: &mut FontContext, lcx: &mut LayoutContext<BrushIndex>, styles: &StyleSet, composing: Option<(usize, &str)>, caret: Option<usize>) -> Laid {
        let range = self.span(line);
        let source = &self.text[self.lines.range(line)];
        let (text, underline) = match composing.and_then(|(at, preedit)| {
            preview_preedit(source, range, at, self.selection.as_ref(), preedit)
        }) {
            Some((shown, underline)) => (Cow::Owned(shown), Some(underline)),
            None => (Cow::Borrowed(source), None),
        };
        let context = self.markdown.as_ref().map(|fonts| self.context(fonts, Some(line)));
        let markdown = context.as_ref().map(|context| (context, self.blocks.get(line), self.local_caret(line, caret)));
        // Native fenced code has one horizontal visual line; math keeps its
        // existing layout. The editor separately clips unwrapped code text.
        let no_wrap = self.fenced_code(line);
        let width = if no_wrap { None } else { self.width };
        let (layout, decor) = text::build(fcx, lcx, styles, &text, underline, width, Alignment::Start, markdown);
        Laid { layout, decor }
    }
}
