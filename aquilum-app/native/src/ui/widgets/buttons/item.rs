use crate::ui::widgets::layout::contract::leaf_widget;
use masonry::accesskit::{Action, Node, Role};
use masonry::core::{
    AccessCtx, MeasureCtx, PaintCtx, PropertiesRef, Widget, WidgetMut, render_text,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Axis, Point, Rect, RoundedRect};
use masonry::layout::{LenReq, Length};

use crate::ui::icons::Icon;
use crate::ui::tokens::size;
use crate::ui::widgets::{Press, Pressed};
use crate::ui::widgets::press::press_events;
use crate::ui::{text, theme};

pub struct ItemButton {
    icon: Icon,
    text: String,
    line: text::Line,
    press: Press,
    active: bool,
}

impl ItemButton {
    pub fn new(icon: Icon, text: impl Into<String>) -> Self {
        ItemButton { icon, text: text.into(), line: text::Line::default(), press: Press::default(), active: false }
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn set_active(this: &mut WidgetMut<'_, Self>, active: bool) {
        if this.widget.active != active {
            this.widget.active = active;
            this.ctx.request_paint_only();
            this.ctx.request_accessibility_update();
        }
    }
}

impl Widget for ItemButton {
    type Action = Pressed;

    press_events!(line);

    fn measure(&mut self, _ctx: &mut MeasureCtx<'_>, _props: &PropertiesRef<'_>, axis: Axis, len_req: LenReq, _cross: Option<Length>) -> Length {
        match (axis, len_req) {
            (Axis::Vertical, _) => Length::px(size::SIZE_32),
            (Axis::Horizontal, LenReq::FitContent(space)) => space,
            (Axis::Horizontal, _) => Length::px(size::SIZE_32),
        }
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        let t = theme::current();
        let bounds = ctx.content_box();
        let hovered = ctx.is_hovered() || self.press.down;
        paint_item_bg(painter, bounds, self.active, hovered);
        let hovered = hovered || self.active;
        let pad = size::SIDEBAR_ITEM_PADDING;
        let slot = size::SIDEBAR_ICON_SIZE;
        let glyph = size::SIZE_16;
        let inset = (slot - glyph) / 2.0;
        let icon_color = if hovered { t.sidebar_icon_hover } else { t.sidebar_icon };
        self.icon.draw(painter, Point::new(bounds.x0 + pad + inset, bounds.center().y - glyph / 2.0), glyph, 1.6, icon_color);

        let text_x = pad + slot + size::SIDEBAR_ITEM_GAP;
        let available = (bounds.width() - text_x - pad).max(0.0);
        let layout = self.line.layout(ctx, &self.text, available, text::Style::ui(slot));
        let origin = Affine::translate((bounds.x0 + text_x, bounds.center().y - slot / 2.0));
        render_text(painter, origin, layout, &[t.sidebar_text.into()], true);
    }

    fn accessibility_role(&self) -> Role {
        Role::Button
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, node: &mut Node) {
        node.set_label(self.text.clone());
        if self.active {
            node.set_selected(true);
        }
        node.add_action(Action::Click);
    }

    leaf_widget!();
}

pub fn paint_item_bg(painter: &mut Painter<'_>, bounds: Rect, active: bool, hovered: bool) {
    let t = theme::current();
    let bg = if active { Some(t.sidebar_item_active_bg) } else { hovered.then_some(t.sidebar_item_hover_bg) };
    if let Some(bg) = bg {
        painter.fill(RoundedRect::from_rect(bounds, size::SIDEBAR_ITEM_RADIUS), bg).draw();
    }
}
