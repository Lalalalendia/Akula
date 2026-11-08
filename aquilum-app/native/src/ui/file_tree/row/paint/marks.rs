use masonry::core::BrushIndex;
use masonry::imaging::Painter;
use masonry::kurbo::{Point, Rect, RoundedRect};
use masonry::parley::{Affinity, Cursor, Layout, Selection};
use crate::ui::theme;
use crate::ui::tokens::size;

/// Paint highlights behind filename glyphs, not over their foreground.
pub(super) fn paint(painter: &mut Painter<'_>, layout: &Layout<BrushIndex>, matches: &[(usize, usize)], origin: Point) {
    let color = theme::current().search_match_bg;
    for &(start, end) in matches {
        let selection = Selection::new(
            Cursor::from_byte_index(layout, start, Affinity::Downstream),
            Cursor::from_byte_index(layout, end, Affinity::Upstream),
        );
        for (rect, _) in selection.geometry(layout) {
            let bounds = Rect::new(rect.x0, rect.y0, rect.x1, rect.y1) + origin.to_vec2();
            painter.fill(RoundedRect::from_rect(bounds, size::ROUNDED_SM), color).draw();
        }
    }
}
