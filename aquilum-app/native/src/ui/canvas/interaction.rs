//! Pointer gestures and the minimal native Canvas edit toolbar.
//! CanvasPage previews a drag, but only Core CanvasSession owns a mutation.
use super::*;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanvasAction {
    AddText(i64, i64),
    MoveNode(String, i64, i64),
    ResizeNode(String, i64, i64),
    RemoveNode(String),
    Undo,
    Redo,
    Save,
}
use masonry::core::{PointerScrollEvent, ScrollDelta};
const CONTROL_WIDTH: f64 = 72.0;
const CONTROL_HEIGHT: f64 = 32.0;
const CONTROL_GAP: f64 = 5.0;
const CONTROL_RIGHT: f64 = 14.0;
const CONTROL_TOP: f64 = 9.0;
const DRAG_THRESHOLD: f64 = 2.0;
pub(super) enum CanvasDrag {
    Pan { last: Point },
    Node { index: usize, from: Point, original: Rect },
    Resize { index: usize, from: Point, original: Rect },
}
impl CanvasPage {
    fn control_rect(&self, index: usize) -> Rect {
        if index == 4 { return self.delete_control_rect(); }
        let count = 4.0;
        let x = (self.size.width - CONTROL_RIGHT - count * CONTROL_WIDTH - (count - 1.0) * CONTROL_GAP).max(0.0)
            + index as f64 * (CONTROL_WIDTH + CONTROL_GAP);
        Rect::new(x, CONTROL_TOP, x + CONTROL_WIDTH, CONTROL_TOP + CONTROL_HEIGHT)
    }
    fn toolbar_action(&self, point: Point) -> Option<CanvasAction> {
        if !self.editable { return None; }
        let index = (0..5).find(|i| self.control_rect(*i).contains(point))?;
        match index {
            0 => {
                let x = ((self.size.width / 2.0 - self.origin.x) / self.zoom).round() as i64;
                let y = ((self.size.height / 2.0 - self.origin.y) / self.zoom).round() as i64;
                Some(CanvasAction::AddText(x, y))
            }
            1 if self.undo => Some(CanvasAction::Undo),
            2 if self.redo => Some(CanvasAction::Redo),
            3 if self.dirty => Some(CanvasAction::Save),
            4 => self.selected.as_ref().map(|id| CanvasAction::RemoveNode(id.clone())),
            _ => None,
        }
    }
    fn node_at(&self, position: Point) -> Option<usize> {
        if !self.editable { return None; }
        self.board.cards.iter().enumerate().rev()
            .find(|(_, card)| card.kind == Kind::Text && self.rect(card.rect).contains(position))
            .map(|(i, _)| i)
    }
    pub(super) fn pointer(&mut self, ctx: &mut EventCtx<'_>, event: &PointerEvent) {
        match event {
            PointerEvent::Down(down) if down.button.is_none_or(|b| b == PointerButton::Primary) => {
                let p = ctx.local_position(down.state.position);
                if let Some(action) = self.toolbar_action(p) {
                    ctx.submit_action::<CanvasAction>(action);
                    ctx.set_handled();
                    return;
                }
                // Disabled toolbar buttons must never start card dragging.
                if self.editable && (0..5).any(|i| self.control_rect(i).contains(p)) {
                    ctx.set_handled();
                    return;
                }
                if let Some(index) = self.resize_target(p) {
                    self.drag = Some(CanvasDrag::Resize { index, from: p, original: self.board.cards[index].rect });
                    ctx.capture_pointer(); ctx.set_handled(); return;
                }
                if down.state.count == 2 {
                    self.fit(); self.drag = None; ctx.request_paint_only(); ctx.set_handled(); return;
                }
                let hit = self.node_at(p);
                self.selected = hit.map(|index| self.board.cards[index].id.clone());
                ctx.request_paint_only();
                self.drag = match hit {
                    Some(index) => Some(CanvasDrag::Node {
                        index, from: p, original: self.board.cards[index].rect,
                    }),
                    None => Some(CanvasDrag::Pan { last: p }),
                };
                ctx.capture_pointer();
                ctx.set_handled();
            }
            PointerEvent::Move(update) => {
                let p = ctx.local_position(update.current.position);
                if self.preview_resize(p) { ctx.request_paint_only(); return; }
                match &mut self.drag {
                    Some(CanvasDrag::Pan { last }) => {
                        self.origin += p - *last;
                        *last = p;
                        ctx.request_paint_only();
                    }
                    Some(CanvasDrag::Node { index, from, original }) => {
                        let delta = (p - *from) / self.zoom;
                        self.board.cards[*index].rect = Rect::new(
                            original.x0 + delta.x, original.y0 + delta.y,
                            original.x1 + delta.x, original.y1 + delta.y,
                        );
                        ctx.request_paint_only();
                    }
                    _ => {}
                }
            }
            PointerEvent::Up(up) => {
                if self.finish_resize(ctx, up) { return; }
                if let Some(CanvasDrag::Node { index, from, original }) = self.drag.take() {
                    let p = ctx.local_position(up.state.position);
                    let delta = (p - from) / self.zoom;
                    // No-op click does not create an undo entry.
                    let travel = p - from;
                    if travel.x.abs() + travel.y.abs() > DRAG_THRESHOLD {
                        let x = (original.x0 + delta.x).round() as i64;
                        let y = (original.y0 + delta.y).round() as i64;
                        ctx.submit_action::<CanvasAction>(CanvasAction::MoveNode(self.board.cards[index].id.clone(), x, y));
                        ctx.set_handled();
                    } else {
                        self.board.cards[index].rect = original;
                    }
                } else { self.drag = None; }
            }
            PointerEvent::Cancel(_) => {
                if self.cancel_resize(ctx) { return; }
                if let Some(CanvasDrag::Node { index, original, .. }) = self.drag.take() {
                    self.board.cards[index].rect = original; ctx.request_paint_only();
                }
            }
            PointerEvent::Scroll(PointerScrollEvent { delta, state, .. }) => {
                let p = ctx.local_position(state.position);
                match delta {
                    ScrollDelta::LineDelta(_, y) => self.zoom_at(p, (f64::from(*y) * 0.12).exp()),
                    ScrollDelta::PixelDelta(d) if state.modifiers.ctrl() || state.modifiers.meta() => {
                        self.zoom_at(p, (d.y * 0.01).exp());
                    }
                    ScrollDelta::PixelDelta(d) => self.origin += Vec2::new(d.x, d.y),
                    _ => return,
                }
                ctx.request_paint_only();
                ctx.set_handled();
            }
            _ => {}
        }
    }
    pub(super) fn paint_controls(&mut self, ctx: &mut PaintCtx<'_>, painter: &mut Painter<'_>) {
        let tm = theme::current();
        for (i, label) in ["+ Text", "Undo", "Redo", "Save", "Delete"].into_iter().enumerate() {
            let rect = self.control_rect(i);
            let enabled = match i { 1 => self.undo, 2 => self.redo, 3 => self.dirty, 4 => self.selected.is_some(), _ => true };
            let shape = RoundedRect::from_rect(rect, 6.0);
            painter.fill(shape, tm.bg_surface).draw();
            painter.stroke(&shape, &Stroke::new(1.0), tm.border_subtle).draw();
            let layout = self.controls[i].layout(ctx, label, CONTROL_WIDTH - 8.0, Style::ui(18.0));
            let color = if enabled { tm.text_primary } else { tm.text_muted };
            let left = rect.x0 + (rect.width() - f64::from(layout.width())) / 2.0;
            render_text(painter, Affine::translate((left, rect.y0 + 7.0)), layout, &[color.into()], true);
        }
    }
    #[cfg(test)]
    pub(crate) fn card_center(&self, id: &str) -> Option<Point> {
        let card = self.board.cards.iter().find(|c| c.id == id)?;
        Some(self.rect(card.rect).center())
    }
    #[cfg(test)]
    pub(crate) fn toolbar_center(&self, index: usize) -> Point {
        self.control_rect(index).center()
    }
}
