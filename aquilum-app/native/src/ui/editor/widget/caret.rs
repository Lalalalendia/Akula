mod cursor;
use cursor::{nearest_boundary, on_boundary};
use super::*;

impl TextEditor {
    pub(super) fn local(&self, offset: usize) -> (usize, usize, Option<&Layout<BrushIndex>>) {
        let i = self.doc.at_offset(offset);
        let p = self.doc.paragraph(i);
        (i, offset.clamp(p.start, p.end()) - p.start, p.laid())
    }

    pub(super) fn laid_out(&self) -> bool {
        self.doc.measured()
    }

    pub(super) fn padding(&self) -> f64 {
        if self.single_line { 0.0 } else { size::CARET_WIDTH }
    }

    pub(super) fn inner_width(&self) -> f64 {
        (self.width - self.padding() * 2.0).max(0.0)
    }

    pub(super) fn shift(&self) -> Vec2 {
        let content = self.doc.paragraph(0).laid().map_or(0.0, |l| f64::from(l.width()));
        let center = if self.centered && self.single_line { ((self.inner_width() - content) / 2.0).max(0.0) } else { 0.0 };
        Vec2::new(self.padding() + center - self.scroll_x, 0.0)
    }

    pub(super) fn caret(&self) -> Rect {
        let (at, offset, affinity) = cursor::target(self);
        self.caret_at(at, offset, affinity)
    }

    pub(super) fn caret_at(&self, offset: usize, compose: usize, affinity: Affinity) -> Rect {
        let (start, end) = self.doc.loaded_range();
        if offset < start || offset > end {
            let y = self.doc.y_of(offset);
            return Rect::new(0.0, y, size::CARET_WIDTH, y + self.doc.line_height()) + self.shift();
        }
        let (i, local, layout) = self.local(offset);
        let Some(layout) = layout else {
            let origin = self.doc.origin(i);
            return Rect::new(origin.x, self.doc.top(i), origin.x + size::CARET_WIDTH, self.doc.bottom(i)) + self.shift();
        };
        let cursor = Cursor::from_byte_index(layout, local + compose, affinity);
        let line = rect(cursor.geometry(layout, size::CARET_WIDTH as f32), 0.0);
        let middle = line.center().y as f32;
        let metrics = layout.lines().map(|l| *l.metrics()).find(|m| m.block_min_coord <= middle && middle < m.block_max_coord);
        let (top, bottom) = metrics.map_or((line.y0, line.y1), |m| (f64::from(m.block_min_coord), f64::from(m.block_max_coord)));
        let half = (bottom - top) / 2.0;
        let origin = self.doc.origin(i);
        let center = (top + bottom) / 2.0 + origin.y;
        let line = line + Vec2::new(origin.x, 0.0);
        Rect::new(line.x0, center - half, line.x0 + size::CARET_WIDTH, center + half)
            + self.shift() - Vec2::new(self.code_offset_at(i), 0.0)
    }

    pub(super) fn reveal_caret(&mut self) {
        if !self.single_line || !self.laid_out() {
            return;
        }
        let inner = (self.inner_width() - size::CARET_WIDTH).max(0.0);
        let x = self.caret().x0 - self.shift().x;
        if x < self.scroll_x {
            self.scroll_x = x;
        } else if x > self.scroll_x + inner {
            self.scroll_x = x - inner;
        }
        let content = f64::from(self.doc.paragraph(0).layout().width());
        self.scroll_x = self.scroll_x.min((content - inner).max(0.0)).max(0.0);
    }

    pub(super) fn hit(&self, point: Point) -> (usize, Affinity) {
        let point = point - self.shift();
        if point.y < 0.0 && !self.single_line {
            return (0, Affinity::Downstream);
        }
        if point.y >= self.doc.height() && !self.single_line {
            return (self.doc.len(), Affinity::Downstream);
        }
        let i = self.doc.at_y(point.y.clamp(0.0, self.doc.height()));
        let p = self.doc.paragraph(i);
        if matches!(p.fold, Fold::Head | Fold::Member) {
            // A folded embed occupies multiple source lines but one visual
            // rectangle. The top half maps before the source, the bottom half
            // after its *last* member line. This also handles a hidden member
            // selected by the height index.
            let Some(head) = (0..=i).rev().find(|&j| self.doc.paragraph(j).fold == Fold::Head) else {
                return (p.start, Affinity::Downstream);
            };
            let below = point.y > (self.doc.top(head) + self.doc.bottom(head)) / 2.0;
            let at = if below {
                self.doc.folded_end(head).saturating_add(1).min(self.doc.len())
            } else {
                self.doc.paragraph(head).start
            };
            return (at, Affinity::Downstream);
        }
        let local = point - self.doc.origin(i) + Vec2::new(self.code_offset_at(i), 0.0);
        let Some(layout) = p.laid() else { return (p.start, Affinity::Downstream) };
        let cursor = Cursor::from_point(layout, local.x as f32, local.y as f32);
        let text = self.doc.slice(p.range());
        (p.start + nearest_boundary(text, cursor.index()), cursor.affinity())
    }

    pub(super) fn step(&self, forward: bool, word: bool) -> (usize, Affinity) {
        let (i, local, layout) = self.local(self.focus);
        let p = self.doc.paragraph(i);
        let Some(layout) = layout else { return (if forward { p.end() } else { p.start }, Affinity::Downstream) };
        if forward && local >= p.len as usize {
            return match (i + 1 < self.doc.line_count()).then(|| self.doc.paragraph(i + 1)) {
                Some(next) => (next.start, Affinity::Downstream),
                None => (self.focus, self.affinity),
            };
        }
        if !forward && local == 0 {
            return match i.checked_sub(1).map(|j| self.doc.paragraph(j)) {
                Some(prev) => (prev.end(), Affinity::Downstream),
                None => (0, Affinity::Downstream),
            };
        }
        let advance = |cursor: Cursor| match (forward, word) {
            (true, false) => cursor.next_visual(layout),
            (false, false) => cursor.previous_visual(layout),
            (true, true) => cursor.next_visual_word(layout),
            (false, true) => cursor.previous_visual_word(layout),
        };
        let text = self.doc.slice(p.range());
        let mut moved = advance(Cursor::from_byte_index(layout, local, self.affinity));
        while !on_boundary(text, moved.index()) {
            let next = advance(moved);
            if next.index() == moved.index() {
                break;
            }
            moved = next;
        }
        (p.start + moved.index(), moved.affinity())
    }

    pub(super) fn vertical(&mut self, down: bool) -> (usize, Affinity) {
        if self.single_line {
            return if down { (self.doc.len(), Affinity::Downstream) } else { (0, Affinity::Downstream) };
        }
        let caret = self.caret();
        self.at_column(if down { caret.y1 + 1.0 } else { caret.y0 - 1.0 })
    }

    fn at_column(&mut self, y: f64) -> (usize, Affinity) {
        let x = *self.column.get_or_insert(self.caret().x0);
        self.hit(Point::new(x, y))
    }

    pub fn page(this: &mut WidgetMut<'_, Self>, down: bool, dy: f64, extend: bool) {
        let editor = &mut *this.widget;
        if !editor.laid_out() {
            return;
        }
        let target = if dy.abs() < 0.5 {
            (if down { editor.doc.len() } else { 0 }, Affinity::Downstream)
        } else {
            let caret = editor.caret();
            editor.at_column((caret.y0 + caret.y1) / 2.0 + dy)
        };
        editor.move_to(target, extend);
        editor.skip_tables(down);
        this.ctx.request_layout();
        this.ctx.request_render();
    }

    pub(super) fn line_edge(&self, end: bool) -> (usize, Affinity) {
        let (i, local, layout) = self.local(self.focus);
        let p = self.doc.paragraph(i);
        let Some(layout) = layout else { return (if end { p.end() } else { p.start }, Affinity::Downstream) };
        let selection = Selection::from_byte_index(layout, local, self.affinity);
        let moved = if end { selection.line_end(layout, false) } else { selection.line_start(layout, false) };
        let focus = moved.focus();
        (p.start + focus.index(), focus.affinity())
    }
}


#[cfg(test)]
mod folded_hit_tests {
    use super::*;
    use masonry::parley::{FontContext, LayoutContext};

    fn check_folded_edges(source: &str, expected_after: usize) {
        let mut editor = TextEditor::new(source).markdown();
        editor.doc.keep = (0.0, 10_000.0);
        editor.doc.layout(
            &mut FontContext::default(),
            &mut LayoutContext::new(),
            &editor.styles,
            Some(600.0),
            None,
            None,
        );
        assert_eq!(editor.doc.paragraph(0).fold, Fold::Head, "source: {source:?}");
        assert_eq!(editor.doc.paragraph(1).fold, Fold::Member);
        assert_eq!(editor.doc.paragraph(2).fold, Fold::Member);
        assert_eq!(
            editor.doc.folded_end(0).saturating_add(1).min(editor.doc.len()),
            expected_after,
            "bottom half must land after the complete folded source",
        );
        let (top, bottom) = (editor.doc.top(0), editor.doc.bottom(0));
        assert!(bottom - top > 4.0, "folded embed has no visible height");
        let hit = |portion: f64| editor.hit(Point::new(12.0, top + (bottom - top) * portion));
        assert_eq!(hit(0.25), (0, Affinity::Downstream));
        assert_eq!(hit(0.50), (0, Affinity::Downstream));
        assert_eq!(hit(0.75), (expected_after, Affinity::Downstream));
    }

    #[test]
    fn clicks_above_and_below_a_folded_table_target_the_right_source_edge() {
        let prefix = "| а | б |\n| --- | --- |\n| 1 | 2 |";
        let with_tail = format!("{prefix}\nпосле");
        check_folded_edges(&with_tail, prefix.len() + 1);
        check_folded_edges(prefix, prefix.len());
    }

    #[test]
    fn clicks_above_and_below_a_folded_formula_target_the_right_source_edge() {
        let prefix = "$$\nx^2\n$$";
        let with_tail = format!("{prefix}\nпосле");
        check_folded_edges(&with_tail, prefix.len() + 1);
    }
}

#[cfg(test)]
#[path = "caret_tests.rs"]
mod ime_preview_tests;
