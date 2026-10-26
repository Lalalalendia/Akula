use std::ops::{Range, RangeInclusive};

use masonry::imaging::Painter;
use masonry::kurbo::Vec2;
use masonry::parley::{Affinity, BoundingBox, Cursor};

use super::{Document, Fold, TextEditor};
use crate::ui::editor::markdown;

fn paint_range_lines(doc: &Document, shown: RangeInclusive<usize>, range: &Range<usize>) -> RangeInclusive<usize> {
    let first = doc.at_offset(range.start).max(*shown.start());
    let last = doc.at_offset(range.end).min(*shown.end());
    first..=last
}

impl TextEditor {
    pub(super) fn paint_range(&self, painter: &mut Painter<'_>, shown: RangeInclusive<usize>, range: Range<usize>, color: masonry::peniko::Color) {
        for i in paint_range_lines(&self.doc, shown, &range) {
            let p = self.doc.paragraph(i);
            if !(matches!(p.fold, Fold::None | Fold::Above) && p.laid().is_some() && p.start <= range.end && p.end() >= range.start) {
                continue;
            }
            let origin = self.doc.origin(i) - Vec2::new(self.code_offset_at(i), 0.0);
            let layout = p.layout();
            let a = range.start.max(p.start) - p.start;
            let b = range.end.min(p.end()) - p.start;
            let range_rects: Vec<_> = markdown::range_rects(layout, a, b)
                .into_iter()
                .map(|(bounds, _)| {
                    super::rect(BoundingBox::new(bounds.x0, bounds.y0, bounds.x1, bounds.y1), 0.0) + origin + self.shift()
                })
                .collect();
            let continuation = if range.end > p.end() && b == p.len as usize {
                let end = Cursor::from_byte_index(layout, p.len as usize, Affinity::Upstream).geometry(layout, 1.0);
                Some(super::rect(BoundingBox::new(end.x0, end.y0, end.x0 + 8.0, end.y1), 0.0) + origin + self.shift())
            } else {
                None
            };
            if let Some(viewport) = self.code_viewport(i) {
                painter.with_fill_clip(viewport, |clipped| {
                    for rect in &range_rects {
                        clipped.fill(*rect, color).draw();
                    }
                    if let Some(rect) = continuation {
                        clipped.fill(rect, color).draw();
                    }
                });
            } else {
                for rect in range_rects {
                    painter.fill(rect, color).draw();
                }
                if let Some(rect) = continuation {
                    painter.fill(rect, color).draw();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eligible(doc: &Document, lines: RangeInclusive<usize>, range: &Range<usize>) -> Vec<usize> {
        lines.filter(|&i| {
            let p = doc.paragraph(i);
            p.start <= range.end && p.end() >= range.start
        }).collect()
    }

    fn next(seed: &mut u64, limit: usize) -> usize {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (*seed >> 32) as usize % limit
    }

    #[test]
    fn paint_range_bounds_match_full_scan_for_unicode_windows_and_boundaries() {
        let note = (0..256).map(|i| match i % 7 {
            0 => String::new(),
            1 => format!("## Заголовок {i}"),
            2 => "```rust".to_owned(),
            3 => format!("let text = \"你好 {i}\";"),
            4 => "```".to_owned(),
            _ => format!("строка {i}: кириллица, 中文 и [[ссылка]]"),
        }).collect::<Vec<_>>().join("\n");
        let offsets = note.char_indices().map(|(i, _)| i).chain([note.len()]).collect::<Vec<_>>();

        for windowed in [false, true] {
            let doc = if windowed { Document::windowed(crop::Rope::from(note.as_str())) } else { Document::new(&note) };
            let count = doc.line_count();
            let mut seed = 0x20_26_10_22_u64;
            for _ in 0..4_096 {
                let first = next(&mut seed, count);
                let last = first + next(&mut seed, count - first);
                let a = offsets[next(&mut seed, offsets.len())];
                let b = offsets[next(&mut seed, offsets.len())];
                let range = a.min(b)..a.max(b);
                let shown = first..=last;
                let slow = eligible(&doc, shown.clone(), &range);
                let fast = eligible(&doc, paint_range_lines(&doc, shown.clone(), &range), &range);
                assert_eq!(fast, slow, "windowed={windowed}, shown={shown:?}, range={range:?}");
            }
        }
    }

    #[test]
    fn paint_range_bounds_skip_unrelated_visible_paragraphs() {
        let note = (0..512).map(|i| format!("строка {i}: content")).collect::<Vec<_>>().join("\n");
        let doc = Document::new(&note);
        let count = doc.line_count();
        let middle = count / 2;
        let range = doc.paragraph(middle).range();
        assert_eq!(paint_range_lines(&doc, 0..=count - 1, &range), middle..=middle);
        assert!(count > 100);
    }

    #[test]
    fn paint_range_bounds_keep_newline_and_end_of_document_cases() {
        let doc = Document::new("начало\n\n```rust\nlet n = 1;\n```\nконец");
        let count = doc.line_count();
        let shown = 0..=count - 1;
        for line in 0..count {
            let p = doc.paragraph(line);
            for range in [p.start..p.start, p.end()..p.end(), p.start..p.end()] {
                assert_eq!(
                    eligible(&doc, paint_range_lines(&doc, shown.clone(), &range), &range),
                    eligible(&doc, shown.clone(), &range),
                    "line={line}, range={range:?}",
                );
            }
        }
        let out_of_bounds = doc.paragraph(count - 1).end() + 1..usize::MAX;
        assert_eq!(
            eligible(&doc, paint_range_lines(&doc, shown.clone(), &out_of_bounds), &out_of_bounds),
            eligible(&doc, shown, &out_of_bounds),
        );
    }
    // Diagnostic only: this measures selecting eligible paragraph indices,
    // not painting pixels, event-to-present latency, or installed UI frame time.
    #[test]
    #[ignore = "opt-in paired highlight range selection A/B, not a speed gate"]
    fn benchmark_editor_highlight_range_abba() {
        use std::hint::black_box;
        use std::time::{Duration, Instant};

        fn checksum(doc: &Document, lines: RangeInclusive<usize>, range: &Range<usize>) -> u64 {
            lines.filter_map(|i| {
                let p = doc.paragraph(i);
                (p.start <= range.end && p.end() >= range.start).then_some((i + 1) as u64)
            }).sum()
        }

        fn micros(d: Duration) -> f64 { d.as_secs_f64() * 1_000_000.0 }

        fn at(times: &[Duration], pct: usize) -> f64 {
            let mut sorted = times.to_vec();
            sorted.sort();
            let i = (sorted.len() * pct).div_ceil(100).saturating_sub(1);
            micros(sorted[i])
        }

        for lines in [30_000usize, 100_000] {
            let source = (0..lines).map(|i| format!("строка {i}: a test line 中文\n")).collect::<String>();
            let doc = Document::new(&source);
            let n = doc.line_count();
            for width in [128usize, 2_048] {
                let first = n / 2;
                let last = (first + width - 1).min(n - 1);
                let shown = first..=last;
                let marks = width.min(256);
                let ranges: Vec<_> = (0..marks).map(|i| {
                    let line = first + i * (last - first + 1) / marks;
                    doc.paragraph(line).range()
                }).collect();

                let reference = || ranges.iter()
                    .map(|range| checksum(&doc, shown.clone(), range)).sum::<u64>();
                let candidate = || ranges.iter()
                    .map(|range| checksum(&doc, paint_range_lines(&doc, shown.clone(), range), range)).sum::<u64>();
                let expected = black_box(reference());
                assert_eq!(candidate(), expected);

                let (mut a, mut b) = (Vec::new(), Vec::new());
                for round in 0..8 {
                    let mut sample = |new: bool| {
                        let start = Instant::now();
                        let result = black_box(if new { candidate() } else { reference() });
                        assert_eq!(result, expected);
                        (start.elapsed(), result)
                    };
                    if round % 2 == 0 {
                        a.push(sample(false).0);
                        b.push(sample(true).0);
                    } else {
                        b.push(sample(true).0);
                        a.push(sample(false).0);
                    }
                }
                println!(
                    "EDITOR_RANGE_AB lines={lines} visible={} marks={marks} old_p50_us={:.2} new_p50_us={:.2} old_p95_us={:.2} new_p95_us={:.2} checksum={expected}",
                    last - first + 1, at(&a, 50), at(&b, 50), at(&a, 95), at(&b, 95),
                );
            }
        }
    }

}
