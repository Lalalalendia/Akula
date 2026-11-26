use crate::ui::widgets::layout::contract::leaf_widget;
use masonry::accesskit::{Action, Node, Role};
use masonry::core::{
    AccessCtx, MeasureCtx, NewWidget, PaintCtx, PropertiesRef, Widget, render_text,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Axis, Line, Point, Stroke};
use masonry::layout::{LenReq, Length};

use crate::ui::tokens::{number, size};
use crate::ui::widgets::{Press, Pressed};
use crate::ui::widgets::press::press_events;
use crate::ui::{text, theme};

pub struct LinkButton {
    label: String,
    line: text::Line,
    press: Press,
}

impl LinkButton {
    pub fn new(label: &str) -> NewWidget<Self> {
        NewWidget::new(LinkButton { label: label.to_owned(), line: text::Line::default(), press: Press::default() })
    }

    fn style() -> text::Style {
        text::Style::ui((f64::from(theme::ui_font_settings().size) * number::FONT_LINE_HEIGHT_TIGHT).round())
    }
}

impl Widget for LinkButton {
    type Action = Pressed;

    press_events!(line);

    fn measure(&mut self, ctx: &mut MeasureCtx<'_>, _props: &PropertiesRef<'_>, axis: Axis, _len_req: LenReq, _cross: Option<Length>) -> Length {
        let style = Self::style();
        Length::px(match axis {
            Axis::Horizontal => text::measure(ctx, &self.label, style).ceil() + size::SPACE_12 * 2.0,
            Axis::Vertical => style.line_height + size::SPACE_8 * 2.0,
        })
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        let t = theme::current();
        let bounds = ctx.content_box();
        let style = Self::style();
        let width = (bounds.width() - size::SPACE_12 * 2.0).max(0.0);
        let enabled = !ctx.is_disabled();
        let color = if enabled { t.text_link } else { t.text_disabled };
        let layout = self.line.layout(ctx, &self.label, width, style);
        let text_width = f64::from(layout.width());
        let origin = Point::new(bounds.x0 + size::SPACE_12, bounds.y0 + size::SPACE_8);
        render_text(painter, Affine::translate(origin.to_vec2()), layout, &[color.into()], true);
        if enabled && (ctx.is_hovered() || self.press.down) {
            let y = origin.y + style.line_height - 2.0;
            let underline = Line::new((origin.x, y), (origin.x + text_width, y));
            painter.stroke(underline, &Stroke::new(1.0), color).draw();
        }
    }

    fn accessibility_role(&self) -> Role {
        Role::Button
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, node: &mut Node) {
        node.set_label(self.label.clone());
        node.add_action(Action::Click);
    }

    leaf_widget!();
}
