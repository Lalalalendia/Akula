use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ChildrenIds, LayoutCtx, MeasureCtx, PaintCtx, PropertiesRef,
    RegisterCtx, Widget,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Axis, Point, Rect, Size};
use masonry::layout::{LenReq, Length};
use masonry::peniko::ImageBrush;

use super::super::theme;

// A standalone image is a regular, read-only document page, not a modal overlay.
pub struct ImagePage {
    image: ImageBrush,
    frame: Rect,
    name: String,
}

const MARGIN: f64 = 32.0;

impl ImagePage {
    pub fn new(image: ImageBrush, name: &str) -> masonry::core::NewWidget<Self> {
        masonry::core::NewWidget::new(Self { image, frame: Rect::ZERO, name: name.to_owned() })
    }
}

impl Widget for ImagePage {
    type Action = masonry::core::NoAction;

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn measure(&mut self, ctx: &mut MeasureCtx<'_>, _props: &PropertiesRef<'_>, axis: Axis, _len_req: LenReq, _cross: Option<Length>) -> Length {
        ctx.context_size().length(axis).unwrap_or(Length::ZERO)
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        let width = f64::from(self.image.image.width);
        let height = f64::from(self.image.image.height);
        let room = Size::new((size.width - MARGIN * 2.0).max(1.0), (size.height - MARGIN * 2.0).max(1.0));
        let ratio = (room.width / width).min(room.height / height).min(1.0);
        let (w, h) = (width * ratio, height * ratio);
        self.frame = Rect::from_origin_size(Point::new((size.width - w) / 2.0, (size.height - h) / 2.0), Size::new(w, h));
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        painter.fill(ctx.content_box(), theme::current().bg_canvas).draw();
        let factor = self.frame.width() / f64::from(self.image.image.width);
        painter.draw_image(&self.image, Affine::translate(self.frame.origin().to_vec2()) * Affine::scale(factor));
    }

    fn accessibility_role(&self) -> Role {
        Role::Image
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, node: &mut Node) {
        node.set_label(self.name.clone());
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
}
