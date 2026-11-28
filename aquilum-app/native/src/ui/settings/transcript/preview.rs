use crate::ui::editor::{draw_image, paint_caption};
use crate::ui::theme;
use crate::ui::widgets::layout::contract::fill_context;
use crate::video::preview::Playback;
use aquilum_core::background_jobs::speech::Cue;
use masonry::accesskit::Role;
use masonry::core::{
    AccessCtx, ChildrenIds, LayoutCtx, NewWidget, PaintCtx, PropertiesMut, PropertiesRef,
    RegisterCtx, Update, UpdateCtx, Widget,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Rect, Size};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

pub(crate) struct Preview {
    playback: Rc<RefCell<Playback>>,
    cues: Vec<Cue>,
}

impl Preview {
    pub(crate) fn new(playback: Rc<RefCell<Playback>>, cues: Vec<Cue>) -> NewWidget<Self> {
        NewWidget::new(Self { playback, cues })
    }
}

impl Widget for Preview {
    type Action = ();

    fill_context!();

    fn layout(&mut self, _: &mut LayoutCtx<'_>, _: &PropertiesRef<'_>, _: Size) {}

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _: &mut PropertiesMut<'_>, event: &Update) {
        if matches!(event, Update::WidgetAdded) {
            ctx.request_anim_frame();
        }
    }

    fn on_anim_frame(&mut self, ctx: &mut UpdateCtx<'_>, _: &mut PropertiesMut<'_>, _: u64) {
        if self.playback.borrow_mut().tick() {
            ctx.request_anim_frame();
        }
        ctx.request_paint_only();
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        let playback = self.playback.borrow();
        let bounds = ctx.content_box();
        painter.fill(bounds, theme::current().bg_overlay).draw();
        let mut frame = bounds;
        if let Some(image) = playback.frame() {
            let w = f64::from(image.image.width);
            let h = f64::from(image.image.height);
            let scale = (bounds.width() / w).min(bounds.height() / h);
            let midpoint = bounds.center();
            frame = Rect::from_center_size(midpoint, (w * scale, h * scale));
            draw_image(painter, image, frame, [0.0, 0.0, 100.0, 100.0]);
        }
        let ms = (playback.position() * 1000.0) as i64;
        if playback.is_open()
            && let Some(cue) = self
                .cues
                .iter()
                .rev()
                .find(|cue| cue.start_ms <= ms && ms < cue.end_ms)
        {
            paint_caption(
                ctx,
                painter,
                frame,
                frame.y1 - 5.0,
                Arc::from(cue.text.as_str()),
            );
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::Video
    }
    fn accessibility(
        &mut self,
        _: &mut AccessCtx<'_>,
        _: &PropertiesRef<'_>,
        _: &mut masonry::accesskit::Node,
    ) {
    }
    fn register_children(&mut self, _: &mut RegisterCtx<'_>) {}
    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
}
