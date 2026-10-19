//! Real Masonry/AccessKit buttons for visible fenced-code headers.
//!
//! Virtual AccessKit-only nodes cannot receive a WidgetId-targeted action in
//! this Masonry version. Use actual child widgets, created only for headers in
//! the visible viewport and removed when the window moves or text changes.
//! Their pointer hit testing is transparent: the existing painted button
//! remains the single mouse action and does not steal text selection.

use super::*;
use masonry::accesskit::{Action, Role};
use masonry::core::{ActionCtx, AccessEvent, NewWidget, WidgetId, WidgetPod};

const MAX_VISIBLE_HEADERS: usize = 128;

#[derive(Debug)]
pub(super) struct CopyPressed;

pub(super) struct CodeCopyButton;

/// A WidgetId is assigned to just one original opening-fence byte offset.
/// Removal and re-creation never aliases the same widget to another block.
pub(super) struct VisibleCopyButton {
    pub(super) opener: usize,
    pub(super) rect: Rect,
    pub(super) widget: WidgetPod<CodeCopyButton>,
}

impl Widget for CodeCopyButton {
    type Action = CopyPressed;

    fn on_text_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &TextEvent,
    ) {
        if let TextEvent::Keyboard(key) = event {
            if key.key == Key::Named(NamedKey::Enter)
                || matches!(&key.key, Key::Character(c) if c == " ")
            {
                // Stop *both* key phases. Otherwise KeyDown bubbles to the
                // editable parent and inserts a newline or space before the
                // child can activate on KeyUp.
                ctx.set_handled();
                if key.state == KeyState::Up {
                    ctx.submit_action::<CopyPressed>(CopyPressed);
                }
            }
        }
    }

    fn on_access_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &AccessEvent,
    ) {
        if ctx.target() == ctx.widget_id() && event.action == Action::Click {
            ctx.submit_action::<CopyPressed>(CopyPressed);
            ctx.set_handled();
        }
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn children_ids(&self) -> ChildrenIds { ChildrenIds::new() }

    fn measure(
        &mut self,
        _ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        _axis: Axis,
        _request: LenReq,
        _cross: Option<Length>,
    ) -> Length {
        Length::px(24.0)
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, _size: Size) {}

    fn accepts_focus(&self) -> bool { true }

    // The parent keeps the exact existing mouse hitbox and hover behavior.
    fn accepts_pointer_interaction(&self) -> bool { false }

    fn accessibility_role(&self) -> Role { Role::Button }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, node: &mut Node) {
        node.set_label(crate::i18n::t("settings.shortcuts.copyCodeBlock"));
        node.add_action(Action::Click);
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        // The icon is drawn by TextEditor::paint_code_header. Only the
        // keyboard focus outline belongs to the semantic child.
        if ctx.is_focus_target() && ctx.is_window_focused() {
            painter.stroke(
                RoundedRect::from_rect(ctx.border_box(), size::ROUNDED_LG),
                &Stroke::new(1.6),
                theme::current().caret_color,
            ).draw();
        }
    }

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if matches!(event, Update::FocusChanged(_) | Update::HoveredChanged(_)) {
            ctx.request_paint_only();
        }
    }
}

impl TextEditor {
    /// Discover only mounted, painted headers that really intersect the
    /// viewport. A 30k-line code body produces at most one button, not 30k.
    fn visible_copy_headers(&self, top: f64, bottom: f64) -> Vec<(usize, Rect)> {
        if self.single_line || self.doc.markdown.is_none() || !self.laid_out()
            || !top.is_finite() || !bottom.is_finite() || bottom <= top
        {
            return Vec::new();
        }
        let first = self.doc.at_y(top.max(0.0));
        let last = self.doc.at_y(bottom.max(0.0)).min(self.doc.line_count().saturating_sub(1));
        let mut visible = Vec::new();
        for line in first..=last {
            if !matches!(self.doc.block(line), Block::Code { first: true, fence: true, .. }) {
                continue;
            }
            if let Some(rect) = self.code_header_button(line)
                && rect.y1 > top && rect.y0 < bottom
            {
                visible.push((self.doc.paragraph(line).start, rect));
                if visible.len() == MAX_VISIBLE_HEADERS {
                    break;
                }
            }
        }
        visible
    }

    /// Compose has the actual window transform; layout does not. Defer child
    /// graph changes to the mutate pass instead of editing the widget arena
    /// during layout or emitting virtual NodeIds without event routing.
    pub(super) fn refresh_access_code_buttons(&mut self, ctx: &mut ComposeCtx<'_>, top: f64, bottom: f64) {
        let visible = self.visible_copy_headers(top, bottom);
        let differs = visible.len() != self.access_code_buttons.len()
            || visible.iter().any(|(opener, rect)| {
                !self.access_code_buttons.iter().any(|entry| entry.opener == *opener && entry.rect == *rect)
            });
        if !differs { return; }

        ctx.mutate_self_later(move |mut root| {
            let mut editor = root.downcast::<TextEditor>();
            let previous = std::mem::take(&mut editor.widget.access_code_buttons);
            let mut kept = Vec::with_capacity(visible.len());
            let mut removed = false;
            for mut entry in previous {
                if let Some((_, rect)) = visible.iter().find(|(opener, _)| *opener == entry.opener) {
                    entry.rect = *rect;
                    kept.push(entry);
                } else {
                    editor.ctx.remove_child(entry.widget);
                    removed = true;
                }
            }
            let mut added = false;
            for (opener, rect) in visible {
                if kept.iter().any(|entry| entry.opener == opener) { continue; }
                kept.push(VisibleCopyButton {
                    opener,
                    rect,
                    widget: NewWidget::new(CodeCopyButton).to_pod(),
                });
                added = true;
            }
            kept.sort_by_key(|entry| entry.opener);
            editor.widget.access_code_buttons = kept;
            if added || removed {
                editor.ctx.children_changed();
            } else {
                editor.ctx.request_layout();
            }
        });
    }

    pub(super) fn layout_access_code_buttons(&mut self, ctx: &mut LayoutCtx<'_>) {
        for entry in &mut self.access_code_buttons {
            ctx.run_layout(&mut entry.widget, entry.rect.size());
            ctx.place_child(&mut entry.widget, entry.rect.origin());
        }
    }

    /// Parent handler for the accessible child's Click/Enter/Space action.
    /// Reject stale or rebound controls rather than copying another block.
    pub(super) fn access_code_action(&mut self, ctx: &mut ActionCtx<'_>, source: WidgetId) {
        let Some((opener, original)) = self.access_code_buttons.iter()
            .find(|entry| entry.widget.id() == source)
            .map(|entry| (entry.opener, entry.rect)) else { return };
        if self.doc.markdown.is_none() || !self.laid_out() { return; }
        // A platform may deliver an accessibility Click after a scroll but
        // before the deferred child-removal pass runs. Never allow an
        // offscreen, previously exposed node to copy stale content.
        let (top, bottom) = super::visible(ctx.window_transform(), ctx.window_size());
        if !top.is_finite() || !bottom.is_finite()
            || original.y1 <= top || original.y0 >= bottom
        {
            return;
        }
        let line = self.doc.at_offset(opener);
        if self.doc.paragraph(line).start != opener || self.code_header_button(line) != Some(original) {
            return;
        }
        let Some(text) = self.fenced_code_body_from_start(opener) else { return };
        ctx.set_clipboard(text);
        self.copied_code = Some((opener, std::time::Instant::now()));
        ctx.request_render();
        ctx.request_anim_frame();
        ctx.set_handled();
    }

    #[cfg(test)]
    pub(in crate::ui::editor) fn visible_code_button_ids(&self) -> Vec<(usize, WidgetId)> {
        self.access_code_buttons.iter().map(|entry| (entry.opener, entry.widget.id())).collect()
    }
}
