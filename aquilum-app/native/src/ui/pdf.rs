//! Native read-only PDF page inside an ordinary Akula document tab.
//! The PDF itself is never converted into an editable Markdown document.
use masonry::accesskit::{Node, Role};
use masonry::core::keyboard::{Key, KeyState, NamedKey};
use masonry::core::{
    AccessCtx, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx, NewWidget, PaintCtx, PointerEvent,
    PointerScrollEvent, PropertiesMut, PropertiesRef, RegisterCtx, ScrollDelta, TextEvent, Widget,
    render_text,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Axis, Point, Rect, Size};
use masonry::layout::{LenReq, Length};
use masonry::peniko::ImageBrush;

use super::text::{Line, Style};
use super::theme;

const HEADER: f64 = 58.0;

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Step(i32),
    First,
    Last,
    Zoom(i32),
    Rotate,
    Scroll(f64),
    Retry,
}

#[derive(Clone)]
pub struct Display {
    pub image: Option<ImageBrush>,
    pub name: String,
    pub current: usize,
    pub total: usize,
    pub zoom: f64,
    pub rotation: i32,
    pub scroll: f64,
    pub loading: bool,
    pub error: Option<String>,
}

pub struct PdfPage {
    display: Display,
    frame: Rect,
    header: Line,
    helper: Line,
}

impl PdfPage {
    pub fn new(display: Display) -> NewWidget<Self> {
        NewWidget::new(Self {
            display,
            frame: Rect::ZERO,
            header: Line::default(),
            helper: Line::default(),
        })
    }

    fn title(&self) -> String {
        format!(
            "PDF  {}  —  {}/{}  —  {}%  —  {}°",
            self.display.name,
            self.display.current + 1,
            self.display.total.max(1),
            (self.display.zoom * 100.0).round() as i32,
            self.display.rotation * 90,
        )
    }

    fn hint(&self) -> String {
        if let Some(error) = &self.display.error {
            return format!("{error}   (R — повторить)");
        }
        if self.display.loading {
            return "Открытие страницы PDF…".to_owned();
        }
        "← → страницы   ·   колёсико — прокрутка   ·   + / − масштаб   ·   R — поворот"
            .to_owned()
    }
}


fn fit_page(intrinsic: Size, viewport: Size, zoom: f64, scroll: f64) -> Rect {
    let width = (viewport.width - 48.0).max(1.0) * zoom.clamp(0.35, 2.5);
    let height = width * (intrinsic.height / intrinsic.width.max(1.0));
    let body_height = (viewport.height - HEADER).max(1.0);
    let x = (viewport.width - width) / 2.0;
    let y = HEADER + if height < body_height { (body_height - height) / 2.0 }
        else { -scroll.clamp(0.0, (height - body_height).max(0.0)) };
    Rect::from_origin_size(Point::new(x, y), Size::new(width, height))
}

impl Widget for PdfPage {
    type Action = Action;

    fn on_text_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &TextEvent,
    ) {
        let TextEvent::Keyboard(key) = event else { return };
        if key.state != KeyState::Down {
            return;
        }
        let action = match &key.key {
            Key::Named(NamedKey::ArrowRight | NamedKey::PageDown) => Some(Action::Step(1)),
            Key::Named(NamedKey::ArrowLeft | NamedKey::PageUp) => Some(Action::Step(-1)),
            Key::Named(NamedKey::Home) => Some(Action::First),
            Key::Named(NamedKey::End) => Some(Action::Last),
            Key::Character(value) if value.as_str() == "+" || value.as_str() == "=" => Some(Action::Zoom(1)),
            Key::Character(value) if value.as_str() == "-" => Some(Action::Zoom(-1)),
            Key::Character(value) if value.as_str().eq_ignore_ascii_case("r") => {
                if self.display.error.is_some() { Some(Action::Retry) } else { Some(Action::Rotate) }
            }
            _ => None,
        };
        if let Some(action) = action {
            ctx.submit_action::<Action>(action);
            ctx.set_handled();
        }
    }

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        match event {
            PointerEvent::Scroll(PointerScrollEvent { delta, .. }) => {
                let dy = match delta {
                    ScrollDelta::PixelDelta(delta) => delta.y / ctx.scale_factor(),
                    ScrollDelta::LineDelta(_, dy) => f64::from(*dy) * 48.0,
                    _ => 0.0,
                };
                if dy.is_finite() && dy.abs() > f64::EPSILON {
                    ctx.submit_action::<Action>(Action::Scroll(-dy));
                    ctx.set_handled();
                }
            }
            PointerEvent::Down(down) => {
                let x = ctx.local_position(down.state.position).x;
                let width = ctx.content_box().width();
                if x < 64.0 {
                    ctx.submit_action::<Action>(Action::Step(-1));
                    ctx.set_handled();
                } else if x > width - 64.0 {
                    ctx.submit_action::<Action>(Action::Step(1));
                    ctx.set_handled();
                }
            }
            _ => {}
        }
    }

    fn accepts_focus(&self) -> bool {
        true
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn measure(
        &mut self,
        ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        axis: Axis,
        _len_req: LenReq,
        _cross: Option<Length>,
    ) -> Length {
        ctx.context_size().length(axis).unwrap_or(Length::ZERO)
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        self.frame = Rect::ZERO;
        if let Some(image) = &self.display.image {
            let intrinsic = Size::new(f64::from(image.image.width), f64::from(image.image.height));
            self.frame = fit_page(intrinsic, size, self.display.zoom, self.display.scroll);
        }
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        let theme = theme::current();
        let bounds = ctx.content_box();
        painter.fill(bounds, theme.bg_canvas).draw();
        painter.fill(Rect::new(0.0, 0.0, bounds.width(), HEADER), theme.bg_surface).draw();
        let title = self.title();
        let line = self.header.layout(ctx, &title, (bounds.width() - 24.0).max(0.0), Style::ui(15.0));
        render_text(painter, Affine::translate((12.0, 8.0)), line, &[theme.text_primary.into()], true);
        let hint = self.hint();
        let line = self.helper.layout(ctx, &hint, (bounds.width() - 24.0).max(0.0), Style::ui(12.0));
        render_text(painter, Affine::translate((12.0, 31.0)), line, &[theme.text_secondary.into()], true);
        if let Some(image) = &self.display.image {
            let clip = Rect::new(0.0, HEADER, bounds.width(), bounds.height());
            painter.with_fill_clip(clip, |painter| {
                let scale = self.frame.width() / f64::from(image.image.width);
                painter.draw_image(image, Affine::translate(self.frame.origin().to_vec2()) * Affine::scale(scale));
            });
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::Image
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, node: &mut Node) {
        let status = if self.display.image.is_some() { " — изображение готово" }
            else if self.display.loading { " — загрузка" } else { " — ошибка" };
        node.set_label(format!("{}{}", self.title(), status));
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn page_fits_available_width_then_zooms_without_clipping_initially() {
        let source = Size::new(1200.0, 1600.0);
        let viewport = Size::new(720.0, 900.0);
        let fit = fit_page(source, viewport, 1.0, 0.0);
        assert_eq!(fit.width(), 672.0);
        assert!(fit.x0 >= 0.0);
        assert!(fit.x1 <= viewport.width);
        let zoomed = fit_page(source, viewport, 1.25, 0.0);
        assert_eq!(zoomed.width(), 840.0);
        assert!(zoomed.width() > fit.width());
        let scrolled = fit_page(source, viewport, 1.25, 200.0);
        assert!(scrolled.y0 < zoomed.y0);
    }
    #[test]
    fn page_status_has_true_page_counter() {
        let state = Display {
            image: None, name: "file.pdf".into(), current: 4, total: 10,
            zoom: 1.0, rotation: 1, scroll: 0.0, loading: false, error: None,
        };
        let page = PdfPage { display: state, frame: Rect::ZERO, header: Line::default(), helper: Line::default() };
        assert!(page.title().contains("5/10"));
    }
}