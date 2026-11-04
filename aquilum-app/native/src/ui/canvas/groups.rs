//! Group creation and outline-only hit-testing.
//! Group geometry is independent of enclosed nodes; no implicit reparenting.
use super::*;

impl CanvasPage {
    pub(super) fn group_control_rect(&self) -> Rect {
        let x = (self.size.width - 240.0).max(0.0);
        Rect::new(x, 46.0, x + 72.0, 78.0)
    }

    pub(super) fn group_action(&self) -> CanvasAction {
        let x = ((self.size.width / 2.0 - self.origin.x) / self.zoom).round() as i64 - 210;
        let y = ((self.size.height / 2.0 - self.origin.y) / self.zoom).round() as i64 - 130;
        CanvasAction::AddGroup(x, y)
    }

    pub(super) fn group_outline_at(&self, position: Point) -> Option<usize> {
        if !self.editable { return None; }
        self.board.cards.iter().enumerate().rev()
            .find(|(_, card)| {
                if card.kind != Kind::Group { return false; }
                let rect = self.rect(card.rect);
                if !rect.contains(position) { return false; }
                let distance = (position.x - rect.x0).min(rect.x1 - position.x)
                    .min(position.y - rect.y0).min(rect.y1 - position.y);
                distance <= 9.0
            })
            .map(|(index, _)| index)
    }

    #[cfg(test)]
    pub(crate) fn group_border_point(&self, id: &str) -> Option<Point> {
        let card = self.board.cards.iter().find(|card| card.id == id && card.kind == Kind::Group)?;
        let rect = self.rect(card.rect);
        Some(Point::new(rect.x0 + 4.0, rect.center().y))
    }
}
