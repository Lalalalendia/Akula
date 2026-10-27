//! Resize handle for a selected text card and live pointer preview.
//! File Gate and undo history live in Core, not this native widget.
use super::*;

const GRIP: f64 = 12.0;
const MIN_WIDTH: f64 = 48.0;
const MIN_HEIGHT: f64 = 32.0;
const MAX_DIMENSION: f64 = 10_000_000.0;

impl CanvasPage {
    fn resize_handle(&self) -> Option<(usize, Rect)> {
        if !self.editable { return None; }
        let id = self.selected.as_deref()?;
        let (index, card) = self.board.cards.iter().enumerate()
            .find(|(_, card)| card.id == id && card.kind == Kind::Text)?;
        let rect = self.rect(card.rect);
        Some((index, Rect::new(rect.x1 - GRIP, rect.y1 - GRIP, rect.x1, rect.y1)))
    }

    pub(super) fn resize_target(&self, point: Point) -> Option<usize> {
        let (index, rect) = self.resize_handle()?;
        rect.contains(point).then_some(index)
    }

    pub(super) fn paint_resize_handle(&self, painter: &mut Painter<'_>) {
        if let Some((_, grip)) = self.resize_handle() {
            painter.fill(grip, theme::current().text_primary).draw();
        }
    }

    fn resize_rect(original: Rect, delta: Vec2) -> Rect {
        let width = (original.width() + delta.x).round().clamp(MIN_WIDTH, MAX_DIMENSION);
        let height = (original.height() + delta.y).round().clamp(MIN_HEIGHT, MAX_DIMENSION);
        Rect::new(original.x0, original.y0, original.x0 + width, original.y0 + height)
    }

    pub(super) fn preview_resize(&mut self, point: Point) -> bool {
        let Some(CanvasDrag::Resize { index, from, original }) = &self.drag else {
            return false;
        };
        let resized = Self::resize_rect(*original, (point - *from) / self.zoom);
        self.board.cards[*index].rect = resized;
        true
    }

    pub(super) fn finish_resize(&mut self, ctx: &mut EventCtx<'_>, up: &masonry::core::PointerButtonEvent) -> bool {
        if !matches!(self.drag.as_ref(), Some(CanvasDrag::Resize { .. })) { return false; }
        let Some(CanvasDrag::Resize { index, from, original }) = self.drag.take() else {
            return false;
        };
        // A click without deliberate travel is not a resize. This matters for
        // valid legacy cards smaller than the minimum for newly edited sizes.
        if (point - from).x.abs() + (point - from).y.abs() <= 2.0 {
            self.board.cards[index].rect = original;
            ctx.request_paint_only(); ctx.set_handled(); return true;
        }
        let point = ctx.local_position(up.state.position);
        let resized = Self::resize_rect(original, (point - from) / self.zoom);
        let width = resized.width() as i64;
        let height = resized.height() as i64;
        if (width as f64, height as f64) != (original.width(), original.height()) {
            ctx.submit_action::<CanvasAction>(CanvasAction::ResizeNode(
                self.board.cards[index].id.clone(), width, height,
            ));
        } else {
            self.board.cards[index].rect = original;
            ctx.request_paint_only();
        }
        ctx.set_handled();
        true
    }

    pub(super) fn cancel_resize(&mut self, ctx: &mut EventCtx<'_>) -> bool {
        if !matches!(self.drag.as_ref(), Some(CanvasDrag::Resize { .. })) { return false; }
        if let Some(CanvasDrag::Resize { index, original, .. }) = self.drag.take() {
            self.board.cards[index].rect = original;
            ctx.request_paint_only();
        }
        true
    }

    #[cfg(test)]
    pub(crate) fn resize_handle_center(&self) -> Option<Point> {
        Some(self.resize_handle()?.1.center())
    }
}
