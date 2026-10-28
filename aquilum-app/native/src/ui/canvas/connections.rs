//! One-shot, pointer-driven text-card connection mode.
//! Only Core CanvasSession owns JSON mutations, history, and File Gate.
use super::*;

impl CanvasPage {
    pub(super) fn connect_control_rect(&self) -> Rect {
        let x = (self.size.width - 163.0).max(0.0);
        Rect::new(x, 46.0, x + 72.0, 78.0)
    }

    pub(super) fn refresh_connection(&mut self) {
        if self.connect_from.as_ref().is_some_and(|id| {
            !self.board.cards.iter().any(|card| &card.id == id && card.kind == Kind::Text)
        }) {
            self.connect_from = None;
        }
    }

    pub(super) fn toggle_connect_mode(&mut self, ctx: &mut EventCtx<'_>, point: Point) -> bool {
        if !self.editable || !self.connect_control_rect().contains(point) { return false; }
        if self.connect_from.is_some() {
            self.connect_from = None;
        } else if let Some(selected) = self.selected.as_ref() {
            if self.board.cards.iter().any(|card| card.id == *selected && card.kind == Kind::Text) {
                self.connect_from = Some(selected.clone());
            }
        }
        ctx.request_paint_only(); ctx.set_handled();
        true
    }

    pub(super) fn finish_connection(&mut self, ctx: &mut EventCtx<'_>, point: Point) -> bool {
        let Some(source) = self.connect_from.take() else { return false; };
        if let Some(target) = self.node_at(point).map(|index| self.board.cards[index].id.clone()) {
            if source != target {
                self.selected = Some(target.clone());
                ctx.submit_action::<CanvasAction>(CanvasAction::AddEdge(source, target));
            }
        }
        ctx.request_paint_only(); ctx.set_handled();
        true
    }
}
