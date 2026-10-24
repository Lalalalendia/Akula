//! Native, read-only JSON Canvas. All pointer gestures affect the camera only.
mod model;
#[cfg(test)]
mod scene_tests;
use model::{Board, Card, Connection, Kind, Side};

use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx, PaintCtx,
    PointerButton, PointerEvent, PointerScrollEvent, PropertiesMut, PropertiesRef,
    RegisterCtx, ScrollDelta, Update, UpdateCtx, Widget, render_text,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Axis, Line as Segment, Point, Rect, RoundedRect, Size, Stroke, Vec2};
use masonry::layout::{LenReq, Length};
use masonry::peniko::Color;

use crate::ui::text::{Line, Style};
use crate::ui::theme;

const MARGIN: f64 = 48.0;
const MIN_ZOOM: f64 = 0.05;
const MAX_ZOOM: f64 = 8.0;

pub struct CanvasPage {
    board: Board,
    size: Size,
    zoom: f64,
    origin: Vec2,
    fitted: bool,
    drag: Option<Point>,
    text: Vec<Vec<Line>>,
    status: Line,
}

impl CanvasPage {
    pub fn new(value: &serde_json::Value) -> masonry::core::NewWidget<Self> {
        let board = Board::from_json(value);
        let text = board.cards.iter().map(|c| c.lines.iter().map(|_| Line::default()).collect()).collect();
        masonry::core::NewWidget::new(Self {
            board, size: Size::ZERO, zoom: 1.0, origin: Vec2::ZERO,
            fitted: false, drag: None, text, status: Line::default(),
        })
    }

    fn fit(&mut self) {
        let bounds = self.board.bounds;
        let width = (self.size.width - 2.0 * MARGIN).max(1.0);
        let height = (self.size.height - 2.0 * MARGIN).max(1.0);
        self.zoom = (width / bounds.width().max(1.0))
            .min(height / bounds.height().max(1.0)).clamp(MIN_ZOOM, MAX_ZOOM);
        self.origin = Vec2::new(
            self.size.width / 2.0 - bounds.center().x * self.zoom,
            self.size.height / 2.0 - bounds.center().y * self.zoom,
        );
    }

    fn screen(&self, p: Point) -> Point {
        Point::new(self.origin.x + p.x * self.zoom, self.origin.y + p.y * self.zoom)
    }

    fn rect(&self, rect: Rect) -> Rect {
        Rect::from_points(self.screen(rect.origin()), self.screen(Point::new(rect.x1, rect.y1)))
    }

    fn zoom_at(&mut self, at: Point, factor: f64) {
        let next = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let ratio = next / self.zoom;
        self.origin = (self.origin - at.to_vec2()) * ratio + at.to_vec2();
        self.zoom = next;
    }

    fn anchor(card: &Card, side: Option<Side>, toward: Point) -> Point {
        let r = card.rect;
        let chosen = side.unwrap_or_else(|| {
            let d = toward - r.center();
            if d.x.abs() * r.height() > d.y.abs() * r.width() {
                if d.x > 0.0 { Side::Right } else { Side::Left }
            } else if d.y > 0.0 { Side::Bottom } else { Side::Top }
        });
        match chosen {
            Side::Top => Point::new(r.center().x, r.y0),
            Side::Right => Point::new(r.x1, r.center().y),
            Side::Bottom => Point::new(r.center().x, r.y1),
            Side::Left => Point::new(r.x0, r.center().y),
        }
    }

    fn draw_arrow(&self, painter: &mut Painter<'_>, tip: Point, from: Point, color: Color) {
        let delta = tip - from;
        let length = (delta.x * delta.x + delta.y * delta.y).sqrt();
        if length < 1.0 { return; }
        let ux = delta.x / length;
        let uy = delta.y / length;
        let size = 10.0;
        let left = Point::new(tip.x - ux * size - uy * size * 0.5, tip.y - uy * size + ux * size * 0.5);
        let right = Point::new(tip.x - ux * size + uy * size * 0.5, tip.y - uy * size - ux * size * 0.5);
        painter.stroke(&Segment::new(left, tip), &Stroke::new(1.7), color).draw();
        painter.stroke(&Segment::new(right, tip), &Stroke::new(1.7), color).draw();
    }

    fn color(raw: Option<&str>, fallback: Color) -> Color {
        let Some(raw) = raw else { return fallback };
        let preset = match raw {
            "1" => Some("#e36d71"), "2" => Some("#dd9b51"),
            "3" => Some("#d2b45d"), "4" => Some("#61aa76"),
            "5" => Some("#569ad2"), "6" => Some("#9478c9"), _ => None,
        };
        let hex = preset.unwrap_or(raw).strip_prefix('#');
        let Some(hex) = hex.filter(|s| matches!(s.len(), 3 | 4 | 6 | 8)) else { return fallback };
        let digits = if hex.len() <= 4 { 1 } else { 2 };
        let channel = |index: usize| -> Option<u8> {
            let start = index * digits;
            let raw = hex.get(start..start + digits)?;
            let value = u8::from_str_radix(raw, 16).ok()?;
            Some(if digits == 1 { value * 17 } else { value })
        };
        let (Some(r), Some(g), Some(b)) = (channel(0), channel(1), channel(2)) else {
            return fallback;
        };
        let a = if matches!(hex.len(), 4 | 8) {
            let Some(a) = channel(3) else { return fallback };
            a
        } else { 255 };
        Color::from_rgba8(r, g, b, a)
    }

    fn draw_connection(&self, painter: &mut Painter<'_>, edge: &Connection, color: Color) {
        let a = &self.board.cards[edge.from];
        let b = &self.board.cards[edge.to];
        let source = self.screen(Self::anchor(a, edge.from_side, b.rect.center()));
        let target = self.screen(Self::anchor(b, edge.to_side, a.rect.center()));
        painter.stroke(&Segment::new(source, target), &Stroke::new(1.7), color).draw();
        if edge.from_arrow { self.draw_arrow(painter, source, target, color); }
        if edge.to_arrow { self.draw_arrow(painter, target, source, color); }
    }
}

impl Widget for CanvasPage {
    type Action = masonry::core::NoAction;

    fn on_pointer_event(&mut self, ctx: &mut EventCtx<'_>, _props: &mut PropertiesMut<'_>, event: &PointerEvent) {
        match event {
            PointerEvent::Down(e) if e.button.is_none_or(|b| b == PointerButton::Primary) => {
                if e.state.count == 2 {
                    self.fit();
                    self.drag = None;
                    ctx.request_paint_only();
                    ctx.set_handled();
                    return;
                }
                self.drag = Some(ctx.local_position(e.state.position));
                ctx.capture_pointer();
            }
            PointerEvent::Move(e) => {
                if let Some(last) = &mut self.drag {
                    let next = ctx.local_position(e.current.position);
                    self.origin += next - *last;
                    *last = next;
                    ctx.request_paint_only();
                }
            }
            PointerEvent::Up(_) | PointerEvent::Cancel(_) => self.drag = None,
            PointerEvent::Scroll(PointerScrollEvent { delta, state, .. }) => {
                let p = ctx.local_position(state.position);
                match delta {
                    ScrollDelta::LineDelta(_, y) => self.zoom_at(p, (f64::from(*y) * 0.12).exp()),
                    ScrollDelta::PixelDelta(d) if state.modifiers.ctrl() || state.modifiers.meta() => {
                        self.zoom_at(p, (d.y * 0.01).exp());
                    }
                    ScrollDelta::PixelDelta(d) => self.origin += Vec2::new(d.x, d.y),
                    _ => return,
                }
                ctx.request_paint_only();
                ctx.set_handled();
            }
            _ => {}
        }
    }

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if matches!(event, Update::FontsChanged) {
            for lines in &mut self.text { for line in lines { line.clear(); } }
            self.status.clear();
            ctx.request_paint_only();
        }
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn measure(&mut self, ctx: &mut MeasureCtx<'_>, _props: &PropertiesRef<'_>, axis: Axis, _len_req: LenReq, _cross: Option<Length>) -> Length {
        ctx.context_size().length(axis).unwrap_or(Length::ZERO)
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        self.size = size;
        ctx.set_clip_path(size.to_rect());
        if !self.fitted && size.width > 0.0 && size.height > 0.0 {
            self.fit();
            self.fitted = true;
        }
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        let tm = theme::current();
        let viewport = ctx.content_box();
        painter.fill(viewport, tm.bg_canvas).draw();
        // Render edges first, cards in file order (last is on top).
        for edge in &self.board.connections {
            let color = Self::color(edge.color.as_deref(), tm.border_subtle);
            self.draw_connection(painter, edge, color);
        }
        let zoom = self.zoom;
        let origin = self.origin;
        for (card, lines) in self.board.cards.iter().zip(&mut self.text) {
            let r = Rect::new(
                origin.x + card.rect.x0 * zoom, origin.y + card.rect.y0 * zoom,
                origin.x + card.rect.x1 * zoom, origin.y + card.rect.y1 * zoom,
            );
            if !r.overlaps(viewport) { continue; }
            let outline = Self::color(card.color.as_deref(), tm.border_subtle);
            let box_shape = RoundedRect::from_rect(r, if card.kind == Kind::Group { 4.0 } else { 8.0 });
            if card.kind != Kind::Group {
                painter.fill(box_shape, tm.bg_surface).draw();
            }
            painter.stroke(&box_shape, &Stroke::new(if card.kind == Kind::Group { 2.0 } else { 1.0 }), outline).draw();
            if r.width() < 48.0 || r.height() < 18.0 { continue; }
            // Use the same world-to-screen transform for text and card geometry:
            // without it, zooming stretches the card but leaves its text at 100%.
            let inset = 12.0 * zoom;
            let line_h = 19.0 * zoom;
            let visible = ((r.height() - inset * 2.0) / line_h).floor().max(0.0) as usize;
            let style = Style::ui(19.0);
            let available_world_width = ((r.width() - 2.0 * inset) / zoom).max(1.0);
            painter.with_fill_clip(r, |painter| {
                for (i, (text, layout)) in card.lines.iter().zip(lines).take(visible).enumerate() {
                    let shaped = layout.layout(ctx, text, available_world_width, style);
                    let at = Affine::translate((r.x0 + inset, r.y0 + inset + i as f64 * line_h))
                        * Affine::scale(zoom);
                    render_text(painter, at, shaped, &[tm.text_primary.into()], true);
                }
            });
        }
        let status = format!(
            "Canvas · {} nodes / {} edges · {:.0}% · read-only{}",
            self.board.total_cards, self.board.total_connections, self.zoom * 100.0,
            if self.board.cards.len() < self.board.total_cards || self.board.connections.len() < self.board.total_connections { " · preview limited" } else { "" }
        );
        let shaped = self.status.layout(ctx, &status, (viewport.width() - 28.0).max(1.0), Style::ui(20.0));
        render_text(painter, Affine::translate((14.0, 12.0)), shaped, &[tm.text_secondary.into()], true);
    }

    fn accessibility_role(&self) -> Role { Role::Canvas }
    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, node: &mut Node) {
        node.set_label(format!("Read-only Canvas: {} nodes, {} edges", self.board.total_cards, self.board.total_connections));
    }
    fn children_ids(&self) -> ChildrenIds { ChildrenIds::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn page() -> CanvasPage {
        let board = Board::from_json(&json!({
            "nodes": [{"id":"a","type":"text","text":"hello",
                "x":-50,"y":20,"width":200,"height":100}]
        }));
        CanvasPage {
            text: board.cards.iter().map(|c| c.lines.iter().map(|_| Line::default()).collect()).collect(),
            board, size: Size::new(800.0, 600.0), zoom: 1.0,
            origin: Vec2::ZERO, fitted: false, drag: None, status: Line::default(),
        }
    }

    #[test]
    fn fit_places_cards_inside_viewport_and_keeps_valid_zoom() {
        let mut page = page();
        page.fit();
        let card = page.rect(page.board.cards[0].rect);
        assert!(card.x0 >= 0.0 && card.y0 >= 0.0);
        assert!(card.x1 <= page.size.width && card.y1 <= page.size.height);
        assert!((MIN_ZOOM..=MAX_ZOOM).contains(&page.zoom));
    }

    #[test]
    fn zoom_preserves_the_world_point_under_cursor() {
        let mut page = page();
        page.fit();
        let cursor = Point::new(280.0, 170.0);
        let before = Point::new(
            (cursor.x - page.origin.x) / page.zoom,
            (cursor.y - page.origin.y) / page.zoom,
        );
        page.zoom_at(cursor, 1.5);
        let after = page.screen(before);
        assert!((after.x - cursor.x).abs() < 1e-9);
        assert!((after.y - cursor.y).abs() < 1e-9);
    }

    #[test]
    fn valid_short_and_long_canvas_colors_include_alpha() {
        let fallback = Color::BLACK;
        assert_eq!(CanvasPage::color(Some("#F0A"), fallback).to_rgba8().r, 255);
        assert_eq!(CanvasPage::color(Some("#F0A8"), fallback).to_rgba8().a, 136);
        assert_eq!(CanvasPage::color(Some("#12AB34CD"), fallback).to_rgba8().a, 205);
        assert_eq!(CanvasPage::color(Some("not-a-color"), fallback), fallback);
    }
}
