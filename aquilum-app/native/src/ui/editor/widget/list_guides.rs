use super::*;

// Restrict redraw work to the visible range plus a small ancestor context.
// Reading all earlier rows for every frame is prohibitive for long documents.
const LOOKBEHIND: usize = 128;
const GUIDE_OPACITY: f32 = 0.35;

#[derive(Clone, Copy)]
struct OpenGuide {
    parent: usize,
    level: usize,
    end: Option<usize>,
}

fn close(active: &mut Vec<OpenGuide>, completed: &mut Vec<(usize, usize)>) {
    while let Some(guide) = active.pop() {
        if let Some(end) = guide.end {
            completed.push((guide.parent, end));
        }
    }
}

impl TextEditor {
    fn list_guide_spans(&self, first: usize, last: usize) -> Vec<(usize, usize)> {
        let count = self.doc.line_count();
        if count == 0 || first >= count || first > last {
            return Vec::new();
        }
        let mut open = Vec::<OpenGuide>::new();
        let mut done = Vec::new();
        for line in first.saturating_sub(LOOKBEHIND)..=last.min(count - 1) {
            if self.doc.paragraph(line).fold == Fold::Member {
                continue;
            }
            let block = self.doc.block(line);
            if let Some(item) = block.item() {
                while open.last().is_some_and(|guide| guide.level >= item.level()) {
                    let guide = open.pop().expect("list guide stack is not empty");
                    if let Some(end) = guide.end {
                        done.push((guide.parent, end));
                    }
                }
                for guide in &mut open {
                    guide.end = Some(line);
                }
                if !item.ordered {
                    open.push(OpenGuide { parent: line, level: item.level(), end: None });
                }
                continue;
            }
            if let Block::Continuation { level, .. } = block {
                for guide in &mut open {
                    if *level > guide.level {
                        guide.end = Some(line);
                    }
                }
                continue;
            }
            let content = self.doc.slice(self.doc.paragraph(line).range());
            if content.trim().is_empty() {
                continue;
            }
            if matches!(block, Block::Code { .. }) {
                let indent = content.bytes().take_while(|b| matches!(*b, b' ' | b'\t')).count();
                let mut nested = false;
                for guide in &mut open {
                    let parent_indent = self.doc.block(guide.parent).item().map_or(0, |item| item.indent());
                    if indent > parent_indent {
                        // The fenced code is still inside the list branch.
                        // It cannot begin a guide by itself without a nested item.
                        if guide.end.is_some() {
                            guide.end = Some(line);
                        }
                        nested = true;
                    }
                }
                if nested {
                    continue;
                }
            }
            close(&mut open, &mut done);
        }
        close(&mut open, &mut done);
        done.retain(|(parent, end)| *parent <= last && *end >= first);
        done.sort_unstable_by_key(|(parent, _)| *parent);
        done
    }

    fn list_guide_rects(&self, first: usize, last: usize) -> Vec<Rect> {
        let Some(fonts) = self.doc.markdown.as_ref() else { return Vec::new() };
        self.list_guide_spans(first, last).into_iter().filter_map(|(parent, end)| {
            let item = self.doc.block(parent).item()?;
            let paragraph = self.doc.paragraph(parent);
            let center = paragraph.laid().map(|layout| {
                let marker = item.marker();
                let left = Cursor::from_byte_index(layout, marker.start, Affinity::Downstream)
                    .geometry(layout, 0.0).x0;
                let right = Cursor::from_byte_index(layout, marker.start + 1, Affinity::Downstream)
                    .geometry(layout, 0.0).x0;
                self.shift().x + self.doc.origin(parent).x + (left + right) / 2.0
            }).unwrap_or_else(|| {
                // A parent before the rendered viewport has no Parley layout.
                self.shift().x + markdown::LIST_START
                    + item.level() as f64 * markdown::LIST_LEVEL + f64::from(fonts.ch) / 2.0
            });
            let middle = paragraph.laid()
                .and_then(|layout| layout.lines().next())
                .map_or((self.doc.bottom(parent) - self.doc.top(parent)) / 2.0, |line| {
                    f64::from(line.metrics().block_min_coord + line.metrics().block_max_coord) / 2.0
                });
            let y0 = (self.shift().y + self.doc.origin(parent).y + middle + 2.5
                + markdown::LIST_START).max(self.shift().y + self.doc.top(first));
            let y1 = (self.shift().y + self.doc.bottom(end))
                .min(self.shift().y + self.doc.bottom(last));
            (y1 > y0).then_some(Rect::new(
                center - size::BORDER_1 / 2.0, y0,
                center + size::BORDER_1 / 2.0, y1,
            ))
        }).collect()
    }

    pub(super) fn paint_list_blocks(&self, painter: &mut Painter<'_>, first: usize, last: usize) {
        self.paint_blocks(painter, first, last);
        let ink = theme::current().editor_list_marker_color.multiply_alpha(GUIDE_OPACITY);
        for guide in self.list_guide_rects(first, last) {
            painter.fill(guide, ink).draw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masonry::app::{RenderRoot, RenderRootOptions, WindowSizePolicy};
    use masonry::core::{NewWidget, WidgetTag};
    use masonry::dpi::PhysicalSize;
    use masonry::theme::default_property_set;
    use std::sync::Arc;

    const TAG: WidgetTag<TextEditor> = WidgetTag::named("list-guides");
    const WIDTH: u32 = 600;
    const HEIGHT: u32 = 400;

    fn with_editor<R>(source: &str, f: impl FnOnce(&TextEditor) -> R) -> R {
        let options = RenderRootOptions {
            default_properties: Arc::new(default_property_set()),
            use_system_fonts: true,
            size_policy: WindowSizePolicy::User,
            size: PhysicalSize::new(WIDTH, HEIGHT),
            scale_factor: 1.0,
            test_font: None,
        };
        let mut root = RenderRoot::new(
            NewWidget::new(TextEditor::new(source).markdown()).with_tag(TAG).erased(),
            |_| {},
            options,
        );
        let _ = root.redraw();
        let editor = root.get_widget_with_tag(TAG).expect("editor").inner();
        f(editor)
    }

    fn spans(source: &str) -> Vec<(usize, usize)> {
        with_editor(source, |editor| {
            editor.list_guide_spans(0, editor.doc.line_count().saturating_sub(1))
        })
    }

    #[test]
    fn flat_and_ordered_lists_do_not_gain_extra_guides() {
        assert!(spans("- one\n- two\n- three").is_empty());
        assert!(spans("1. one\n2. two").is_empty());
    }

    #[test]
    fn nested_unordered_list_ends_before_next_sibling() {
        assert_eq!(spans("- parent\n  - child one\n  - child two\n- sibling"), vec![(0, 2)]);
    }

    #[test]
    fn multiple_levels_produce_parent_and_child_guides() {
        assert_eq!(spans("- A\n  - B\n    - C\n  - D"), vec![(0, 3), (1, 2)]);
    }

    #[test]
    fn blank_lines_do_not_split_loose_lists() {
        assert_eq!(spans("- parent\n\n  - first\n\n  - second"), vec![(0, 4)]);
    }

    #[test]
    fn unindented_code_and_horizontal_rules_break_branches() {
        assert!(spans("- parent\n```rust\nlet x = 1;\n```\n- sibling").is_empty());
        assert_eq!(spans("- parent\n  - child\n---\n- sibling"), vec![(0, 1)]);
    }

    #[test]
    fn partial_viewport_keeps_nearby_parent_without_unbounded_scan() {
        with_editor("- parent\n  - first\n  - second\n  - third", |editor| {
            assert_eq!(editor.list_guide_spans(2, 3), vec![(0, 3)]);
            let rects = editor.list_guide_rects(2, 3);
            assert_eq!(rects.len(), 1);
            assert!(rects[0].y1 > rects[0].y0);
        });
    }

    #[test]
    fn guide_is_visible_in_a_rasterized_frame() {
        let source = "- parent\n  - child one\n  - child two";
        let options = RenderRootOptions {
            default_properties: Arc::new(default_property_set()),
            use_system_fonts: true,
            size_policy: WindowSizePolicy::User,
            size: PhysicalSize::new(WIDTH, HEIGHT),
            scale_factor: 1.0,
            test_font: None,
        };
        let mut root = RenderRoot::new(
            NewWidget::new(TextEditor::new(source).markdown()).with_tag(TAG).erased(),
            |_| {},
            options,
        );
        let (layers, _) = root.redraw();
        let editor = root.get_widget_with_tag(TAG).expect("editor").inner();
        let guides = editor.list_guide_rects(0, 2);
        assert_eq!(guides.len(), 1);
        let guide = guides[0];
        let mut renderer = crate::render::Renderer::new(2);
        renderer.render(&layers, WIDTH as u16, HEIGHT as u16, 1.0, theme::background()).unwrap();
        let pixels = renderer.frame_pixels();
        let x = guide.center().x.floor() as usize;
        let reference = x.saturating_sub(6);
        let from = (guide.y0 + 1.0).ceil().max(0.0) as usize;
        let to = (guide.y1 - 1.0).floor().min(HEIGHT as f64) as usize;
        assert!(reference < x && x < WIDTH as usize && to > from);
        let tinted_rows = (from..to)
            .filter(|&y| pixels[y * WIDTH as usize + x] != pixels[y * WIDTH as usize + reference])
            .count();
        assert!(tinted_rows >= 4, "guide must paint at least four pixels beyond the adjacent background");
    }

    #[test]
    fn guide_geometry_is_nested_and_rendering_does_not_change_markdown() {
        let source = "- A\n  - B\n    - C\n  - D";
        with_editor(source, |editor| {
            let rects = editor.list_guide_rects(0, 3);
            assert_eq!(rects.len(), 2);
            assert!(rects[0].x0 < rects[1].x0);
            assert!(rects.iter().all(|rect| rect.y1 > rect.y0));
            assert_eq!(editor.text(), source);
        });
    }
}
