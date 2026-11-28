mod page;
pub use page::ImagePage;
use crate::ui::widgets::layout::contract::fill_context;
use masonry::accesskit::{Node, Role};
use masonry::core::keyboard::{Key, KeyState, NamedKey};
use masonry::core::{
    AccessCtx, ChildrenIds, EventCtx, Layer, LayoutCtx, NewWidget, PaintCtx, PointerEvent, PropertiesMut, PropertiesRef, RegisterCtx, TextEvent, Update, UpdateCtx, Widget,
};
use super::widgets::{Intro, NORMAL};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Rect, RoundedRect, Size};
use masonry::peniko::ImageBrush;

use super::menu::box_shadow;
use super::theme;
const MARGIN: f64 = 32.0;
const RADIUS: f64 = 8.0;

#[derive(Debug)]
pub struct ViewerClosed;

pub struct Viewer {
    image: ImageBrush,
    frame: Rect,
    intro: Intro,
}

impl Viewer {
    pub fn new(image: ImageBrush) -> NewWidget<Self> {
        NewWidget::new(Viewer { image, frame: Rect::ZERO, intro: Intro::new(NORMAL) })
    }
}

impl Widget for Viewer {
    fn on_anim_frame(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, interval: u64) {
        if self.intro.advance(interval) {
            ctx.request_anim_frame();
        }
        ctx.set_opacity(self.intro.value() as f32);
    }

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if matches!(event, Update::WidgetAdded) {
            ctx.set_opacity(0.0);
            ctx.request_anim_frame();
        }
    }
    type Action = ViewerClosed;

    fn on_text_event(&mut self, ctx: &mut EventCtx<'_>, _props: &mut PropertiesMut<'_>, event: &TextEvent) {
        if let TextEvent::Keyboard(k) = event
            && k.state == KeyState::Down
            && k.key == Key::Named(NamedKey::Escape)
        {
            ctx.submit_action::<ViewerClosed>(ViewerClosed);
        }
    }

    fn accepts_focus(&self) -> bool {
        true
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fill_context!();

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        let (iw, ih) = (f64::from(self.image.image.width), f64::from(self.image.image.height));
        let room = Size::new((size.width - MARGIN * 2.0).max(1.0), (size.height - MARGIN * 2.0).max(1.0));
        let k = (room.width / iw).min(room.height / ih).min(1.0);
        let (w, h) = (iw * k, ih * k);
        self.frame = Rect::new((size.width - w) / 2.0, (size.height - h) / 2.0, (size.width + w) / 2.0, (size.height + h) / 2.0);
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        let tm = theme::current();
        painter.fill(ctx.content_box(), tm.bg_overlay).draw();
        let shape = RoundedRect::from_rect(self.frame, RADIUS);
        box_shadow(painter, shape, tm.shadow_action);
        let k = self.frame.width() / f64::from(self.image.image.width);
        let image = &self.image;
        let origin = self.frame.origin();
        painter.with_fill_clip(shape, |painter| painter.draw_image(image, Affine::translate(origin.to_vec2()) * Affine::scale(k)));
    }

    fn accessibility_role(&self) -> Role {
        Role::Dialog
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, node: &mut Node) {
        node.set_modal();
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }

    fn as_layer(&mut self) -> Option<&mut dyn Layer> {
        Some(self)
    }
}

impl Layer for Viewer {
    fn capture_pointer_event(&mut self, ctx: &mut EventCtx<'_>, _props: &mut PropertiesMut<'_>, event: &PointerEvent) {
        if let PointerEvent::Down(_) = event {
            ctx.submit_action::<ViewerClosed>(ViewerClosed);
        }
    }
}
