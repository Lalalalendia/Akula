use super::*;

const ROW_HEIGHT: f64 = 36.0;
const PANEL_WIDTH: f64 = 360.0;
const PANEL_HEIGHT: f64 = 448.0;
const HEADER_HEIGHT: f64 = 44.0;

pub(super) struct Panel {
    pub open: bool,
    scroll: f64,
    rect: Rect,
    title: Line,
    lines: Vec<Line>,
}

impl Panel {
    pub fn new() -> Self {
        Self { open: false, scroll: 0.0, rect: Rect::ZERO, title: Line::default(), lines: Vec::new() }
    }

    fn rows(&self) -> Rect {
        Rect::new(self.rect.x0 + size::SPACE_8, self.rect.y0 + HEADER_HEIGHT, self.rect.x1 - size::SPACE_8, self.rect.y1 - size::SPACE_8)
    }

    fn max_scroll(&self, count: usize) -> f64 {
        (count as f64 * ROW_HEIGHT - self.rows().height().max(0.0)).max(0.0)
    }

    fn row_at(&self, point: Point, count: usize) -> Option<usize> {
        if !self.rows().contains(point) { return None; }
        let index = ((point.y - self.rows().y0 + self.scroll) / ROW_HEIGHT).floor() as usize;
        (index < count).then_some(index)
    }

    fn row_rect(&self, index: usize) -> Rect {
        let rect = self.rows();
        let top = rect.y0 + index as f64 * ROW_HEIGHT - self.scroll;
        Rect::new(rect.x0, top, rect.x1, top + ROW_HEIGHT)
    }
}

impl ReaderView {
    pub(super) fn layout_toc(&mut self, ctx: &mut LayoutCtx<'_>, window: Size) {
        if let Some(button) = &mut self.contents_button {
            let dimensions = Size::new(size::SIZE_32, size::SIZE_32);
            let top = ((self.header - size::SIZE_32) / 2.0).round();
            let close_x = window.width - size::SPACE_12 - size::SIZE_32;
            ctx.run_layout(button, dimensions);
            ctx.place_child(button, Point::new(close_x - (size::SIZE_32 + size::SPACE_4) * 2.0, top));
        }
        let width = PANEL_WIDTH.min((window.width - size::SPACE_24).max(0.0));
        let top = self.header + size::SPACE_4;
        let height = PANEL_HEIGHT.min((window.height - top - size::SPACE_12 - PROGRESS).max(0.0));
        self.toc.rect = Rect::from_origin_size(Point::new((window.width - width - size::SPACE_12).max(0.0), top), Size::new(width, height));
        self.toc.scroll = self.toc.scroll.clamp(0.0, self.toc.max_scroll(self.session.contents().len()));
    }

    pub fn toggle_contents(this: &mut WidgetMut<'_, Self>) {
        this.widget.toc.open = !this.widget.toc.open;
        this.widget.toc.scroll = 0.0;
        this.ctx.request_layout();
    }

    pub(super) fn toc_key(&mut self, ctx: &mut EventCtx<'_>, command: Option<Command>) -> bool {
        if !self.toc.open { return false; }
        if command == Some(Command::Close) {
            self.toc.open = false;
            ctx.request_paint_only();
        }
        ctx.set_handled();
        true
    }

    pub(super) fn toc_pointer(&mut self, ctx: &mut EventCtx<'_>, event: &PointerEvent) -> bool {
        if !self.toc.open { return false; }
        match event {
            PointerEvent::Down(down) => {
                let point = ctx.local_position(down.state.position);
                // Header buttons must still work while the TOC is open.
                if point.y < self.header { return false; }
                if self.toc.rect.contains(point) {
                    if let Some(index) = self.toc.row_at(point, self.session.contents().len()) {
                        let moved = self.session.jump_to_contents(index);
                        self.toc.open = false;
                        self.toc.scroll = 0.0;
                        self.marks = None;
                        self.selection = None;
                        if moved { self.moved(ctx, true); }
                    }
                } else {
                    self.toc.open = false;
                }
                ctx.request_paint_only();
            }
            PointerEvent::Scroll(PointerScrollEvent { delta, state, .. }) => {
                let point = ctx.local_position(state.position);
                if self.toc.rect.contains(point) {
                    let amount = match delta {
                        ScrollDelta::PixelDelta(d) => -d.y / ctx.scale_factor(),
                        ScrollDelta::LineDelta(_, y) => -f64::from(*y) * crate::ui::scroll::LINE,
                        ScrollDelta::PageDelta(_, y) => -f64::from(*y) * self.toc.rows().height(),
                    };
                    if amount.is_finite() {
                        self.toc.scroll = (self.toc.scroll + amount).clamp(0.0, self.toc.max_scroll(self.session.contents().len()));
                        ctx.request_paint_only();
                    }
                }
            }
            PointerEvent::Move(_) | PointerEvent::Leave(_) => {
                self.set_hover(ctx, None);
                self.set_quote_hover(ctx, None);
            }
            _ => {}
        }
        ctx.set_handled();
        true
    }

    pub(super) fn draw_toc(&mut self, ctx: &mut PaintCtx<'_>, painter: &mut Painter<'_>) {
        if !self.toc.open || self.toc.rect.width() <= 0.0 || self.toc.rect.height() <= HEADER_HEIGHT { return; }
        let tm = theme::current();
        let panel = self.toc.rect;
        let shape = RoundedRect::from_rect(panel, size::RADIUS_XL);
        box_shadow(painter, shape, tm.elevation_float);
        painter.fill(shape, tm.dialog_bg).draw();
        painter.stroke(&RoundedRect::from_rect(panel.inflate(-0.5, -0.5), size::RADIUS_XL - 0.5), &Stroke::new(size::BORDER_1), tm.border_subtle).draw();
        let line = Self::line_height();
        let title = self.toc.title.layout(ctx, &t("reader.contents"), (panel.width() - size::SPACE_24).max(1.0), Style::ui(line));
        render_text(painter, Affine::translate((panel.x0 + size::SPACE_12, panel.y0 + (HEADER_HEIGHT - line) / 2.0)), title, &[tm.text_primary.into()], true);
        let rows = self.toc.rows();
        let first = (self.toc.scroll / ROW_HEIGHT).floor() as usize;
        let count = self.session.contents().len();
        let last = ((self.toc.scroll + rows.height()) / ROW_HEIGHT).ceil() as usize;
        let last = last.saturating_add(1).min(count);
        self.toc.lines.resize_with(last.saturating_sub(first), Line::default);
        painter.with_fill_clip(rows, |painter| {
            for index in first..last {
                let entry = &self.session.contents()[index];
                let slot = index - first;
                let item = self.toc.row_rect(index);
                if entry.section == self.session.section() {
                    painter.fill(item, tm.bg_surface_subtle).draw();
                }
                let indent = (entry.depth.min(6) as f64 * size::SPACE_12).min((item.width() - size::SPACE_48).max(0.0));
                let width = (item.width() - indent - size::SPACE_16).max(1.0);
                let layout = self.toc.lines[slot].layout(ctx, &entry.title, width, Style::ui(line));
                render_text(painter, Affine::translate((item.x0 + size::SPACE_8 + indent, item.y0 + (ROW_HEIGHT - line) / 2.0)), layout, &[tm.text_primary.into()], true);
            }
        });
        let max = self.toc.max_scroll(count);
        if max > 0.0 && rows.height() > 0.0 {
            let height = (rows.height() * rows.height() / (count as f64 * ROW_HEIGHT)).clamp(size::SPACE_12, rows.height());
            let top = rows.y0 + (rows.height() - height) * self.toc.scroll / max;
            painter.fill(Rect::new(rows.x1 - size::SPACE_2, top, rows.x1, top + height), tm.text_secondary.multiply_alpha(0.6)).draw();
        }
    }

    #[cfg(test)]
    pub fn test_toc_row(&self, index: usize) -> Option<Rect> {
        let row = self.toc.row_rect(index);
        (self.toc.open && index < self.session.contents().len() && row.y0 >= self.toc.rows().y0 - 0.1 && row.y1 <= self.toc.rows().y1 + 0.1).then_some(row)
    }

    #[cfg(test)]
    pub fn test_toc_scroll(&self) -> Option<f64> {
        self.toc.open.then_some(self.toc.scroll)
    }

    #[cfg(test)]
    pub fn test_toc_bounds(&self) -> Option<Rect> {
        self.toc.open.then_some(self.toc.rect)
    }
}
