//! Native selection helpers, with Core CanvasSession owning all mutations.
use super::*;

impl CanvasPage {
    pub(super) fn delete_control_rect(&self) -> Rect {
        let x = (self.size.width - 86.0).max(0.0);
        Rect::new(x, 46.0, x + 72.0, 78.0)
    }

    pub(super) fn refresh_selection(&mut self) {
        if self.selected.as_ref().is_some_and(|id| {
            !self.board.cards.iter().any(|card| &card.id == id)
        }) {
            self.selected = None;
        }
    }
}
