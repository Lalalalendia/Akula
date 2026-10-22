use masonry::core::{PaintCtx, Widget};
use masonry::imaging::Painter;
use masonry::kurbo::{Axis, Rect};
use super::area::ScrollArea;
impl<W: Widget> ScrollArea<W> {
    pub(super) fn paint_find_marks(&mut self, ctx: &mut PaintCtx<'_>, painter: &mut Painter<'_>, span: &super::bar::Span) {
        // Ticks are purely decorative and stay in the free inset to the right
        // of the scrollbar thumb. No pointer behavior is changed.
        if self.axis != Axis::Vertical || self.find_marks.is_empty() {
            return;
        }
        let scale = ctx.scale_factor();
        if !scale.is_finite() || scale <= 0.0 {
            return;
        }
        let geometry = (span.track.height(), scale);
        if self.find_raster != Some(geometry) {
            self.find_rows = super::find_marks::marker_rows(&self.find_marks, geometry.0, geometry.1);
            self.find_raster = Some(geometry);
        }
        let left = (span.track.x1 - 3.0).max(span.track.x0);
        let top = (span.track.y0 * scale).round() / scale;
        let ink = crate::ui::theme::current().bg_accent;
        for &row in &self.find_rows {
            let y = top + row as f64 / scale;
            if y + 1.0 / scale <= span.track.y1 + 0.5 / scale {
                painter.fill(Rect::new(left, y, span.track.x1, y + 1.0 / scale), ink).draw();
            }
        }
    }
}
