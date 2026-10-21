use masonry::accesskit::{Action, Node, Role};
use masonry::core::keyboard::KeyState;
use masonry::core::{
    AccessCtx, AccessEvent, ChildrenIds, CursorIcon, EventCtx, LayoutCtx, MeasureCtx, PaintCtx,
    PointerButton, PointerEvent, PropertiesMut, PropertiesRef, QueryCtx, RegisterCtx, TextEvent,
    Update, UpdateCtx, Widget, WidgetMut, render_text,
};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Axis, Point, Rect, Size, Vec2};
use masonry::layout::{LenReq, Length};
use masonry::parley::{Affinity, Cursor, Selection};

use super::Paint;
use crate::ui::keymap::{self, Command, Scope};
use crate::ui::tokens::size;
use crate::ui::{text, theme};

const NAME_DRAG_THRESHOLD: f64 = 4.0;
const NAME_FEEDBACK_SECONDS: f64 = 1.5;

fn short_click(start: Point, end: Point, count: u8, dragged: bool, selection_collapsed: bool) -> bool {
    count == 1 && !dragged && selection_collapsed
        && (end.x - start.x).abs().max((end.y - start.y).abs()) < NAME_DRAG_THRESHOLD
}

/// Source filenames can exceed the rendered ellipsis layout.
fn visible_len(layout: &masonry::parley::Layout<masonry::core::BrushIndex>, source_len: usize) -> usize {
    layout.lines().flat_map(|line| line.runs()).flat_map(|run| run.clusters())
        .map(|cluster| cluster.text_range().end).max().unwrap_or(0).min(source_len)
}

/// Emitted only by the filename caption. The App checks the active file
/// identity and reports clipboard success before showing confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionAction {
    CopyFileName,
}

pub struct Caption {
    text: String,
    line: text::Line,
    feedback_line: text::Line,
    paint: Paint,
    trailing: bool,
    copyable: bool,
    selection: Selection,
    select_all: bool,
    pressed: Option<(Point, u8, bool)>,
    feedback: Option<(bool, f64)>,
}

impl Caption {
    pub fn new(text: &str) -> Self {
        Caption {
            text: text.to_owned(),
            line: text::Line::default(),
            feedback_line: text::Line::default(),
            paint: |t| t.text_tertiary,
            trailing: false,
            copyable: false,
            selection: Selection::default(),
            select_all: false,
            pressed: None,
            feedback: None,
        }
    }

    pub fn trailing(text: &str, paint: Paint) -> Self {
        Caption { paint, trailing: true, ..Caption::new(text) }
    }

    /// Only the active document filename may opt in; other Captions retain
    /// their inert, nonselectable behavior.
    pub fn copyable_file_name(mut self) -> Self {
        self.copyable = true;
        self
    }

    pub fn set_text(this: &mut WidgetMut<'_, Self>, text: &str) {
        if this.widget.text != text {
            this.widget.text = text.to_owned();
            this.widget.line.clear();
            this.widget.selection = Selection::default();
            this.widget.select_all = false;
            this.widget.pressed = None;
            this.widget.feedback = None;
            this.ctx.request_paint_only();
            this.ctx.request_accessibility_update();
        }
    }

    pub fn set_copy_feedback(this: &mut WidgetMut<'_, Self>, success: bool) {
        if !this.widget.copyable {
            return;
        }
        this.widget.feedback = Some((success, NAME_FEEDBACK_SECONDS));
        this.widget.selection = Selection::default();
        this.widget.select_all = false;
        this.ctx.request_paint_only();
        this.ctx.request_accessibility_update();
        this.ctx.request_anim_frame();
    }

    /// Hit testing and selection use the exact centered/ellipsized layout
    /// painted by the existing caption renderer, not a differently shaped label.
    fn displayed(&self, bounds: Rect) -> Option<(&masonry::parley::Layout<masonry::core::BrushIndex>, usize, Point)> {
        let (layout, end) = self.line.visible_selection_layout(bounds.width(), &self.text)?;
        let room = (bounds.width() - f64::from(layout.width())).max(0.0);
        let x = bounds.x0 + if self.trailing { room } else { room / 2.0 };
        let y = bounds.center().y - size::SIZE_20 / 2.0;
        Some((layout, end, Point::new(x, y)))
    }

    fn hit_name(&self, bounds: Rect, pos: Point) -> bool {
        self.displayed(bounds).is_some_and(|(layout, _, origin)| {
            pos.x >= origin.x && pos.x <= origin.x + f64::from(layout.width())
                && pos.y >= bounds.y0 && pos.y <= bounds.y1
        })
    }

    fn copy_selected(&self, ctx: &mut EventCtx<'_>) {
        if self.select_all {
            ctx.set_clipboard(self.text.clone());
            return;
        }
        let Some((_, visible_end, _)) = self.displayed(ctx.content_box()) else { return };
        let range = self.selection.text_range();
        let end = range.end.min(visible_end);
        if let Some(text) = self.text.get(range.start.min(end)..end).filter(|s| !s.is_empty()) {
            ctx.set_clipboard(text.to_owned());
        }
    }
}

impl Widget for Caption {
    type Action = CaptionAction;

    fn on_pointer_event(&mut self, ctx: &mut EventCtx<'_>, _props: &mut PropertiesMut<'_>, event: &PointerEvent) {
        if !self.copyable || self.text.is_empty() {
            return;
        }
        let bounds = ctx.content_box();
        match event {
            PointerEvent::Down(e) if e.button == Some(PointerButton::Primary) => {
                let at = ctx.local_position(e.state.position);
                if !self.hit_name(bounds, at) {
                    return;
                }
                let selection = {
                    let Some((layout, _, origin)) = self.displayed(bounds) else { return };
                    let x = (at.x - origin.x) as f32;
                    let y = (at.y - origin.y) as f32;
                    match e.state.count {
                        1 => Selection::from_point(layout, x, y),
                        2 => Selection::word_from_point(layout, x, y),
                        _ => Selection::new(
                            Cursor::from_byte_index(layout, 0, Affinity::Downstream),
                            Cursor::from_byte_index(layout, visible_len(layout, self.text.len()), Affinity::Upstream),
                        ),
                    }
                };
                self.selection = selection;
                self.select_all = false;
                self.pressed = Some((at, e.state.count, false));
                self.feedback = None;
                ctx.request_focus();
                ctx.capture_pointer();
                ctx.request_paint_only();
                ctx.set_handled();
            }
            PointerEvent::Move(update) if self.pressed.is_some() && ctx.is_active() => {
                let at = ctx.local_position(update.current.position);
                if let Some((start, _, dragged)) = self.pressed.as_mut() {
                    *dragged |= (at.x - start.x).abs().max((at.y - start.y).abs()) >= NAME_DRAG_THRESHOLD;
                }
                if let Some((layout, _, origin)) = self.displayed(bounds) {
                    self.selection = self.selection.extend_to_point(
                        layout, (at.x - origin.x) as f32, (at.y - origin.y) as f32,
                    );
                    ctx.request_paint_only();
                }
                ctx.set_handled();
            }
            PointerEvent::Up(e) if e.button == Some(PointerButton::Primary) => {
                let down = self.pressed.take();
                let at = ctx.local_position(e.state.position);
                if let Some((start, count, dragged)) = down {
                    let is_click = short_click(start, at, count, dragged, self.selection.is_collapsed())
                        && self.hit_name(bounds, at);
                    if is_click {
                        ctx.submit_action::<CaptionAction>(CaptionAction::CopyFileName);
                    }
                    ctx.set_handled();
                }
            }
            PointerEvent::Cancel(_) => self.pressed = None,
            _ => {}
        }
    }

    fn on_text_event(&mut self, ctx: &mut EventCtx<'_>, _props: &mut PropertiesMut<'_>, event: &TextEvent) {
        if !self.copyable {
            return;
        }
        let TextEvent::Keyboard(key) = event else { return };
        if key.state != KeyState::Down {
            return;
        }
        match keymap::command(&[Scope::Text], &key.key, key.modifiers) {
            Some(Command::Copy) => {
                self.copy_selected(ctx);
                ctx.set_handled();
            }
            Some(Command::SelectAll) => {
                if let Some((layout, _, _)) = self.displayed(ctx.content_box()) {
                    self.selection = Selection::new(
                        Cursor::from_byte_index(layout, 0, Affinity::Downstream),
                        Cursor::from_byte_index(layout, visible_len(layout, self.text.len()), Affinity::Upstream),
                    );
                    self.select_all = true;
                    ctx.request_paint_only();
                    ctx.set_handled();
                }
            }
            _ => {}
        }
    }

    fn on_access_event(&mut self, ctx: &mut EventCtx<'_>, _props: &mut PropertiesMut<'_>, event: &AccessEvent) {
        if self.copyable && event.action == Action::Click && !self.text.is_empty() {
            ctx.submit_action::<CaptionAction>(CaptionAction::CopyFileName);
        }
    }

    fn get_cursor(&self, _ctx: &QueryCtx<'_>, _pos: Point) -> CursorIcon {
        if self.copyable { CursorIcon::Text } else { CursorIcon::Default }
    }

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        match event {
            Update::FontsChanged => {
                self.line.clear();
                self.feedback_line.clear();
                ctx.request_paint_only();
            }
            Update::FocusChanged(false) if self.copyable => {
                self.selection = Selection::default();
                self.select_all = false;
                ctx.request_paint_only();
            }
            _ => {}
        }
    }

    fn on_anim_frame(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, interval: u64) {
        if let Some((success, remaining)) = self.feedback {
            let remaining = remaining - interval as f64 / 1e9;
            self.feedback = (remaining > 0.0).then_some((success, remaining));
            ctx.request_paint_only();
            if self.feedback.is_some() {
                ctx.request_anim_frame();
            } else {
                ctx.request_accessibility_update();
            }
        }
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn measure(&mut self, _ctx: &mut MeasureCtx<'_>, _props: &PropertiesRef<'_>, axis: Axis, _len_req: LenReq, _cross: Option<Length>) -> Length {
        match axis {
            Axis::Vertical => Length::px(size::SIZE_20),
            Axis::Horizontal => Length::ZERO,
        }
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, _size: Size) {}

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        let bounds = ctx.content_box();
        let style = text::Style::ui(size::SIZE_20);
        if let Some((success, _)) = self.feedback {
            let key = if success { "editor.fileNameCopied" } else { "editor.fileNameCopyFailed" };
            let notice = crate::i18n::t(key);
            let layout = self.feedback_line.layout(ctx, &notice, bounds.width(), style);
            let x = bounds.x0 + (bounds.width() - f64::from(layout.width())).max(0.0) / 2.0;
            let y = bounds.center().y - size::SIZE_20 / 2.0;
            render_text(painter, Affine::translate((x, y)), layout, &[theme::current().text_secondary.into()], true);
            return;
        }
        let layout = self.line.layout(ctx, &self.text, bounds.width(), style);
        let room = (bounds.width() - f64::from(layout.width())).max(0.0);
        let x = bounds.x0 + if self.trailing { room } else { room / 2.0 };
        let y = bounds.center().y - size::SIZE_20 / 2.0;
        if self.copyable && !self.selection.is_collapsed() {
            let origin = Vec2::new(x, y);
            for (rect, _) in self.selection.geometry(layout) {
                painter.fill(Rect::new(rect.x0, rect.y0, rect.x1, rect.y1) + origin, theme::current().selection_bg).draw();
            }
        }
        render_text(painter, Affine::translate((x, y)), layout, &[(self.paint)(&theme::current()).into()], true);
    }

    fn accessibility_role(&self) -> Role {
        if self.copyable { Role::Button } else { Role::Label }
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, node: &mut Node) {
        if self.copyable {
            let label = match self.feedback {
                Some((true, _)) => crate::i18n::t("editor.fileNameCopied"),
                Some((false, _)) => crate::i18n::t("editor.fileNameCopyFailed"),
                None => format!("{} — {}", self.text, crate::i18n::t("editor.copyFileName")),
            };
            node.set_label(label);
            node.add_action(Action::Click);
        }
        node.set_value(self.text.clone());
    }

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{short_click, Caption};
    use std::{cell::RefCell, rc::Rc, sync::Arc};
    use masonry::app::{RenderRoot, RenderRootOptions, RenderRootSignal, WindowSizePolicy};
    use masonry::core::keyboard::{Code, Key, KeyState, KeyboardEvent, Location};
    use masonry::core::{NewWidget, PointerButton, PointerButtonEvent, PointerEvent, PointerId,
        PointerInfo, PointerState, PointerType, PointerUpdate, TextEvent};
    use masonry::dpi::{PhysicalPosition, PhysicalSize};
    use masonry::kurbo::Point;
    use masonry::theme::default_property_set;
    use masonry::widgets::Flex;

    const WIDTH: u32 = 440;
    const HEIGHT: u32 = 40;
    const NAME: &str = "chapter-2026-测试.md";

    #[test]
    fn only_an_unmoved_single_click_copies_full_filename() {
        let p = Point::new(60.0, 10.0);
        assert!(short_click(p, p, 1, false, true));
        assert!(!short_click(p, Point::new(75.0, 10.0), 1, false, true));
        assert!(!short_click(p, p, 1, true, true));
        assert!(!short_click(p, p, 2, false, true));
        assert!(!short_click(p, p, 1, false, false));
    }

    #[test]
    fn selecting_a_filename_substring_does_not_require_copying_the_full_path() {
        let signals = Rc::new(RefCell::new(Vec::<RenderRootSignal>::new()));
        let sink = Rc::clone(&signals);
        let opts = RenderRootOptions {
            default_properties: Arc::new(default_property_set()),
            use_system_fonts: true,
            size_policy: WindowSizePolicy::User,
            size: PhysicalSize::new(WIDTH, HEIGHT),
            scale_factor: 1.0,
            test_font: None,
        };
        let header = NewWidget::new(Flex::row().with(NewWidget::new(Caption::new(NAME).copyable_file_name()), 1.0));
        let mut root = RenderRoot::new(header.erased(),
            move |signal| sink.borrow_mut().push(signal), opts);
        let _ = root.redraw();

        let mouse = PointerInfo { pointer_id: Some(PointerId::PRIMARY),
            persistent_device_id: None, pointer_type: PointerType::Mouse };
        let mut state = PointerState { position: PhysicalPosition::new(205.0, 10.0),
            count: 1, scale_factor: 1.0, ..PointerState::default() };
        state.buttons.insert(PointerButton::Primary);
        root.handle_pointer_event(PointerEvent::Down(PointerButtonEvent {
            pointer: mouse, button: Some(PointerButton::Primary), state: state.clone(),
        }));
        state.position = PhysicalPosition::new(258.0, 10.0);
        root.handle_pointer_event(PointerEvent::Move(PointerUpdate {
            pointer: mouse, current: state.clone(), coalesced: vec![], predicted: vec![],
        }));
        state.buttons.remove(PointerButton::Primary);
        root.handle_pointer_event(PointerEvent::Up(PointerButtonEvent {
            pointer: mouse, button: Some(PointerButton::Primary), state,
        }));
        let key = KeyboardEvent {
            state: KeyState::Down, key: Key::Character("c".into()), code: Code::Unidentified,
            location: Location::Standard, modifiers: crate::test_dir::command(), repeat: false,
            is_composing: false,
        };
        root.handle_text_event(TextEvent::Keyboard(key));
        let _ = root.redraw();
        let copied: Vec<String> = signals.borrow().iter().filter_map(|signal| match signal {
            RenderRootSignal::ClipboardStore(value) => Some(value.clone()),
            _ => None,
        }).collect();
        assert_eq!(copied.len(), 1);
        assert!(!copied[0].is_empty(), "a real drag must select some text");
        assert!(NAME.contains(&copied[0]), "selected range must come from the displayed filename");
        assert_ne!(copied[0], NAME, "drag-select cannot silently copy the complete filename");
    }
}
