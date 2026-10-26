//! Read-only selected DICOM slice in a normal Akula tab.
use crate::dicom::Study;
use crate::ui::{text::{Line, Style}, theme};
use masonry::accesskit::{Node, Role};
use masonry::core::keyboard::{Key, KeyState, NamedKey};
use masonry::core::{render_text, AccessCtx, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx,
    NewWidget, PaintCtx, PropertiesMut, PropertiesRef, RegisterCtx, TextEvent, Update, UpdateCtx, Widget};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Axis, Point, Rect, Size};
use masonry::layout::{LenReq, Length};
use masonry::peniko::{ImageAlphaType, ImageBrush};

const HEADER: f64 = 56.0;
const MARGIN: f64 = 24.0;

pub struct DicomPage {
    study: Study,
    image: ImageBrush,
    frame: Rect,
    line: Line,
    failure: Option<String>,
}

impl DicomPage {
    pub fn new(study: Study) -> Result<NewWidget<Self>, String> {
        let frame = study.render()?;
        let image = crate::render::from_rgba(frame.rgba, frame.width, frame.height, ImageAlphaType::Alpha)
            .ok_or("DICOM: не удалось создать изображение")?;
        Ok(NewWidget::new(Self { study, image, frame: Rect::ZERO,
            line: Line::default(), failure: None }))
    }

    fn move_selection(&mut self, ctx: &mut EventCtx<'_>, frame_delta: Option<i32>) {
        let before = self.study.clone();
        let changed = match frame_delta {
            Some(delta) => self.study.advance_frame(delta),
            None => false,
        };
        if !changed { return; }
        match self.study.render().and_then(|f| {
            crate::render::from_rgba(f.rgba, f.width, f.height, ImageAlphaType::Alpha)
                .ok_or_else(|| "DICOM: не удалось создать изображение".to_owned())
        }) {
            Ok(image) => { self.image = image; self.failure = None; }
            Err(e) => { self.study = before; self.failure = Some(e); }
        }
        self.line.clear();
        ctx.request_layout();
    }

    fn move_series(&mut self, ctx: &mut EventCtx<'_>, delta: i32) {
        let old = self.study.clone();
        if !self.study.advance_series(delta) { return }
        match self.study.render().and_then(|f| {
            crate::render::from_rgba(f.rgba, f.width, f.height, ImageAlphaType::Alpha)
                .ok_or_else(|| "DICOM: не удалось создать изображение".to_owned())
        }) {
            Ok(image) => { self.image = image; self.failure = None; }
            Err(e) => { self.study = old; self.failure = Some(e); }
        }
        self.line.clear();
        ctx.request_layout();
    }
}

impl Widget for DicomPage {
    type Action = masonry::core::NoAction;

    fn on_text_event(&mut self, ctx: &mut EventCtx<'_>, _props: &mut PropertiesMut<'_>, event: &TextEvent) {
        let TextEvent::Keyboard(k) = event else { return };
        if k.state != KeyState::Down { return; }
        match &k.key {
            Key::Named(NamedKey::ArrowRight | NamedKey::ArrowDown) => self.move_selection(ctx, Some(1)),
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowUp) => self.move_selection(ctx, Some(-1)),
            Key::Named(NamedKey::PageDown) => self.move_series(ctx, 1),
            Key::Named(NamedKey::PageUp) => self.move_series(ctx, -1),
            _ => return,
        };
        ctx.set_handled();
    }

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if matches!(event, Update::FontsChanged) {
            self.line.clear();
            ctx.request_layout();
        }
    }

    fn accepts_focus(&self) -> bool { true }
    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn measure(&mut self, ctx: &mut MeasureCtx<'_>, _props: &PropertiesRef<'_>,
               axis: Axis, _len_req: LenReq, _cross: Option<Length>) -> Length {
        ctx.context_size().length(axis).unwrap_or(Length::ZERO)
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        let image_width = f64::from(self.image.image.width);
        let image_height = f64::from(self.image.image.height);
        let room = Size::new((size.width - 2.0*MARGIN).max(1.0),
                             (size.height - HEADER - 2.0*MARGIN).max(1.0));
        let scale = (room.width / image_width).min(room.height / image_height).min(1.0);
        let w = image_width * scale;
        let h = image_height * scale;
        self.frame = Rect::from_origin_size(
            Point::new((size.width - w)/2.0, HEADER + (size.height - HEADER - h)/2.0),
            Size::new(w, h));
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        let theme = theme::current();
        let bounds = ctx.content_box();
        painter.fill(bounds, theme.bg_canvas).draw();
        let label = self.failure.as_deref().map_or_else(|| self.study.label(), |e| format!("DICOM: {e}"));
        let layout = self.line.layout(ctx, &label, (bounds.width() - 2.0*MARGIN).max(1.0), Style::ui(16.0));
        render_text(painter, Affine::translate((MARGIN, 18.0)), layout, &[theme.text_primary.into()], true);
        let scale = self.frame.width() / f64::from(self.image.image.width);
        painter.draw_image(&self.image,
            Affine::translate(self.frame.origin().to_vec2()) * Affine::scale(scale));
    }

    fn accessibility_role(&self) -> Role { Role::Image }
    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, node: &mut Node) {
        node.set_label(self.study.label());
    }
    fn children_ids(&self) -> ChildrenIds { ChildrenIds::new() }
}
