use std::ops::Range;

use masonry::core::{BrushIndex, StyleSet};
use masonry::parley::{Alignment, AlignmentOptions, FontContext, LayoutContext};

use crate::ui::editor::markdown::Block;
use super::super::Document;
use super::super::laid::PLAIN;

impl Document {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn measure(&mut self, fcx: &mut FontContext, lcx: &mut LayoutContext<BrushIndex>, styles: &StyleSet, compose: Option<(usize, &str)>, caret: Option<usize>, rebreak: bool, spans: &[Range<usize>], pinned: &[usize], stale: Option<&Range<usize>>, wanted: &mut Vec<String>) -> bool {
        let keep = self.kept();
        let mut line = self.line_height();
        let mut changed = false;
        // Test-only counters; ordinary editor builds do not profile layouts.
        #[cfg(test)]
        let tracing = aq_trace::enabled();
        #[cfg(test)]
        let (mut built, mut reused, mut skipped, mut rebroken) = (0usize, 0usize, 0usize, 0usize);
        #[cfg(test)]
        let (mut shape_time, mut rebreak_time) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
        let (len, mut next) = (self.lines.count(), 0);
        let indices = spans.iter().flat_map(|r| r.clone()).filter(|&i| {
            let fresh = i >= next && i < len;
            next = next.max(i + 1);
            fresh
        });
        for i in indices {
            let range = self.span(i);
            let composing = compose.filter(|(at, _)| range.contains(at) || *at == range.end);
            let moved = self.markdown.is_some() && self.local_caret(i, caret) != self.laid.get(i).map_or(&PLAIN, |laid| &laid.decor).caret;
            let (current, no_wrap) = (composing.is_none() && !moved, rebreak && self.fenced_code(i));
            match self.laid.get_mut(i).filter(|_| current) {
                Some(laid) => {
                    #[cfg(test)]
                    if tracing {
                        reused += 1;
                    }
                    if rebreak {
                        #[cfg(test)]
                        let begun = tracing.then(std::time::Instant::now);
                        let width = self.width.map(|w| (w - laid.decor.inset as f32).max(0.0));
                        laid.layout.break_all_lines(if no_wrap { None } else { width });
                        laid.layout.align(Alignment::Start, AlignmentOptions::default());
                        self.heights.set_text(i, laid.height());
                        #[cfg(test)]
                        if let Some(begun) = begun {
                            rebroken += 1;
                            rebreak_time += begun.elapsed();
                        }
                    }
                }
                None => {
                    let near = self.placed && self.heights.near(i, keep) || pinned.contains(&i);
                    if current && !near {
                        #[cfg(test)]
                        if tracing {
                            skipped += 1;
                        }
                        if stale.is_some_and(|stale| stale.contains(&i)) {
                            let height = self.estimate(i, line);
                            changed |= self.heights.set_text(i, height);
                        }
                    } else {
                        #[cfg(test)]
                        let begun = tracing.then(std::time::Instant::now);
                        let laid = self.lay(i, fcx, lcx, styles, composing, caret);
                        #[cfg(test)]
                        if let Some(begun) = begun {
                            built += 1;
                            shape_time += begun.elapsed();
                        }
                        changed |= self.heights.set_text(i, laid.height());
                        if *self.blocks.get(i) == Block::Text && laid.layout.len() == 1 {
                            line = laid.height();
                        }
                        wanted.extend(laid.decor.links.iter().filter(|l| l.wiki).map(|l| l.target.trim().to_owned()));
                        if near {
                            self.laid.insert(i, laid);
                        }
                    }
                }
            }
        }
        self.line = line;
        #[cfg(test)]
        if tracing {
            aq_trace::counter("akula: paragraph built", built as f64);
            aq_trace::counter("akula: paragraph reused", reused as f64);
            aq_trace::counter("akula: paragraph skipped", skipped as f64);
            aq_trace::counter("akula: paragraph rebroken", rebroken as f64);
            aq_trace::counter("akula: paragraph shape micros", shape_time.as_secs_f64() * 1e6);
            aq_trace::counter("akula: paragraph rebreak micros", rebreak_time.as_secs_f64() * 1e6);
        }
        changed
    }
}
