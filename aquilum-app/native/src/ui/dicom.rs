//! Non-blocking, strictly read-only DICOM page.
//! Initial DICOMDIR indexing and each selected-frame decode run off the UI thread.
use crate::dicom::{DicomWorker, Navigation};
use crate::ui::{text::{Line, Style}, theme};
use masonry::accesskit::{Node, Role};
use masonry::core::keyboard::{Key, KeyState, NamedKey};
use masonry::core::{
    render_text, AccessCtx, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx, NewWidget,
    PaintCtx, PointerEvent, PointerScrollEvent, PropertiesMut, PropertiesRef,
    RegisterCtx, ScrollDelta, TextEvent, Update, UpdateCtx, Widget,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Axis, Point, Rect, Size};
use masonry::layout::{LenReq, Length};
use masonry::peniko::{ImageAlphaType, ImageBrush};
use std::path::PathBuf;

const HEADER: f64 = 56.0;
const MARGIN: f64 = 24.0;
const WHEEL_STEP: f64 = 80.0;

pub struct DicomPage {
    worker: DicomWorker,
    image: Option<ImageBrush>,
    frame: Rect,
    line: Line,
    label: String,
    failure: Option<String>,
    /// Includes one pending initial load and at most one queued navigation.
    outstanding: usize,
    wheel: f64,
}

impl DicomPage {
    pub fn new(vault: PathBuf, source: PathBuf) -> NewWidget<Self> {
        NewWidget::new(Self {
            worker: DicomWorker::start(vault, source),
            image: None,
            frame: Rect::ZERO,
            line: Line::default(),
            label: "DICOM: открытие исследования...".into(),
            failure: None,
            outstanding: 1,
            wheel: 0.0,
        })
    }

    fn request(&mut self, ctx: &mut EventCtx<'_>, action: Navigation) {
        // Do not let a slow optical reader lock the UI or build a deep backlog.
        if self.outstanding >= 2 || !self.worker.request(action) { return; }
        self.outstanding += 1;
        ctx.request_anim_frame();
    }

    fn display_label(&self) -> String {
        match &self.failure {
            Some(error) => format!("{} · ошибка чтения: {error}", self.label),
            None if self.outstanding > 0 && self.image.is_none() => self.label.clone(),
            None => format!("{} · колесо мыши — срез", self.label),
        }
    }

    fn refresh(&mut self, ctx: &mut UpdateCtx<'_>) {
        // Drain bounded response channel; render only the newest completed frame.
        for _ in 0..2 {
            match self.worker.poll() {
                Ok(Some(update)) => {
                    self.outstanding = self.outstanding.saturating_sub(1);
                    self.label = update.label;
                    self.failure = update.error;
                    if let Some(frame) = update.frame {
                        match crate::render::from_rgba(
                            frame.rgba, frame.width, frame.height, ImageAlphaType::Alpha
                        ) {
                            Some(image) => self.image = Some(image),
                            None => self.failure = Some("DICOM: ошибка создания RGBA изображения".into()),
                        }
                    }
                    self.line.clear();
                    ctx.request_layout();
                    ctx.request_accessibility_update();
                    ctx.request_paint_only();
                }
                Ok(None) => break,
                Err(error) => {
                    if self.outstanding > 0 {
                        self.outstanding = 0;
                        self.failure.get_or_insert_with(|| error.to_owned());
                        self.line.clear();
                        ctx.request_accessibility_update();
                        ctx.request_paint_only();
                    }
                    break;
                }
            }
        }
        if self.outstanding > 0 { ctx.request_anim_frame(); }
    }
}

impl Widget for DicomPage {
    type Action = masonry::core::NoAction;

    fn on_text_event(&mut self, ctx: &mut EventCtx<'_>, _props: &mut PropertiesMut<'_>, event: &TextEvent) {
        let TextEvent::Keyboard(key) = event else { return };
        if key.state != KeyState::Down { return; }
        match &key.key {
            Key::Named(NamedKey::ArrowRight | NamedKey::ArrowDown) =>
                self.request(ctx, Navigation::Frame(1)),
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowUp) =>
                self.request(ctx, Navigation::Frame(-1)),
            Key::Named(NamedKey::PageDown) =>
                self.request(ctx, Navigation::Series(1)),
            Key::Named(NamedKey::PageUp) =>
                self.request(ctx, Navigation::Series(-1)),
            _ => return,
        }
        ctx.set_handled();
    }

    fn on_pointer_event(&mut self, ctx: &mut EventCtx<'_>, _props: &mut PropertiesMut<'_>, event: &PointerEvent) {
        if let PointerEvent::Scroll(PointerScrollEvent { delta, .. }) = event {
            let vertical = match delta {
                ScrollDelta::PixelDelta(p) => p.y / ctx.scale_factor(),
                ScrollDelta::LineDelta(_, y) => f64::from(*y) * 24.0,
                _ => 0.0,
            };
            self.wheel += -vertical;
            if self.wheel.abs() >= WHEEL_STEP {
                self.request(ctx, Navigation::Frame(if self.wheel > 0.0 { 1 } else { -1 }));
                self.wheel = 0.0;
            }
            ctx.set_handled();
        }
    }

    fn on_anim_frame(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, _interval: u64) {
        self.refresh(ctx);
    }

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if matches!(event, Update::WidgetAdded) {
            ctx.request_anim_frame();
        }
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
        let Some(image) = &self.image else {
            self.frame = Rect::ZERO;
            return;
        };
        let width = f64::from(image.image.width);
        let height = f64::from(image.image.height);
        let room = Size::new((size.width - 2.0 * MARGIN).max(1.0),
                             (size.height - HEADER - 2.0 * MARGIN).max(1.0));
        let k = (room.width / width).min(room.height / height).min(1.0);
        let (w, h) = (width * k, height * k);
        self.frame = Rect::from_origin_size(
            Point::new((size.width - w) / 2.0, HEADER + (size.height - HEADER - h) / 2.0),
            Size::new(w, h),
        );
    }
    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        let theme = theme::current();
        let bounds = ctx.content_box();
        painter.fill(bounds, theme.bg_canvas).draw();
        let label = self.display_label();
        let layout = self.line.layout(ctx, &label, (bounds.width() - 2.0 * MARGIN).max(1.0), Style::ui(16.0));
        render_text(painter, Affine::translate((MARGIN, 18.0)), layout, &[theme.text_primary.into()], true);
        if let Some(image) = &self.image {
            let scale = self.frame.width() / f64::from(image.image.width);
            painter.draw_image(image,
                Affine::translate(self.frame.origin().to_vec2()) * Affine::scale(scale));
        }
    }
    fn accessibility_role(&self) -> Role { Role::Image }
    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, node: &mut Node) {
        node.set_label(self.display_label());
    }
    fn children_ids(&self) -> ChildrenIds { ChildrenIds::new() }
}
