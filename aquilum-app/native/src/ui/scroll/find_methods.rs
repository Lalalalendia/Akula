use masonry::core::{Widget, WidgetMut};
use super::area::ScrollArea;
impl<W: Widget> ScrollArea<W> {
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

}
