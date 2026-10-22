use masonry::core::{Widget, WidgetMut};
use super::area::ScrollArea;

impl<W: Widget> ScrollArea<W> {
    pub fn offset(&self) -> f64 {
        self.offset
    }

    pub fn at(mut self, offset: f64) -> Self {
        self.offset = offset.max(0.0);
        self
    }

    pub fn fill(mut self, fill: bool) -> Self {
        self.fill = fill;
        self
    }

    pub fn content_height(&self) -> f64 {
        self.content
    }

    pub fn set_find_marks(this: &mut WidgetMut<'_, Self>, marks: Vec<f64>) {
        if this.widget.find_marks != marks {
            this.widget.find_marks = marks;
            this.widget.find_raster = None;
            this.ctx.request_post_paint();
        }
    }

    #[cfg(test)]
    pub(crate) fn find_marker_fractions(&self) -> &[f64] {
        &self.find_marks
    }
}
