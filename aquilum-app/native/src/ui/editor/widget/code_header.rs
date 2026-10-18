use super::*;
use crate::ui::icons;

const COPY_BUTTON: f64 = 24.0;
const COPY_GLYPH: f64 = 16.0;
const COPY_MARGIN: f64 = 8.0;
const COPIED_FEEDBACK: std::time::Duration = std::time::Duration::from_millis(1_500);

impl TextEditor {
    /// The opening fence remains the header's language label: no synthetic
    /// line, document edit, reflow, cursor displacement or Frame/Nest rewrite.
    /// Hide the control when a long info string occupies the right edge.
    fn code_header_button(&self, index: usize) -> Option<Rect> {
        if self.single_line || !self.laid_out() || self.doc.markdown.is_none() {
            return None;
        }
        let paragraph = self.doc.paragraph(index);
        // Reject non-opening lines before asking which block contains them:
        // a hover over the middle of a 50,000-line fence must be O(1), not
        // scan backwards through the full loaded run on every pointer move.
        if !matches!(paragraph.fold, Fold::None | Fold::Above)
            || !matches!(paragraph.block, Block::Code { first: true, fence: true, .. })
            || self.fenced_code_start(index) != Some(paragraph.start)
        {
            return None;
        }
        let layout = paragraph.laid()?;
        let panel_x = self.shift().x;
        let panel_width = self.inner_width();
        if panel_width < 96.0 || self.doc.bottom(index) - self.doc.top(index) < COPY_BUTTON + 4.0 {
            return None;
        }
        let right = panel_x + panel_width - COPY_MARGIN;
        let left = right - COPY_BUTTON;
        // Only paint if the icon does not overlap the original Markdown info.
        let text_right = panel_x + self.doc.origin(index).x + f64::from(layout.width());
        if text_right + COPY_MARGIN > left {
            return None;
        }
        let line = layout.lines().next()?;
        let metrics = line.metrics();
        let line_top = self.doc.origin(index).y + f64::from(metrics.block_min_coord);
        let line_height = f64::from(metrics.block_max_coord - metrics.block_min_coord);
        let y0 = (line_top + (line_height - COPY_BUTTON) / 2.0)
            .clamp(self.doc.top(index) + 2.0, self.doc.bottom(index) - COPY_BUTTON - 2.0);
        Some(Rect::new(left, y0, right, y0 + COPY_BUTTON))
    }

    fn code_header_at(&self, point: Point) -> Option<usize> {
        let index = self.index_at(point)?;
        let button = self.code_header_button(index)?;
        button.contains(point).then(|| self.doc.paragraph(index).start)
    }

    pub(super) fn code_header_press(&mut self, ctx: &mut EventCtx<'_>, point: Point) -> bool {
        let Some(opener) = self.code_header_at(point) else { return false };
        let Some(code) = self.fenced_code_body_from_start(opener) else { return false };
        ctx.set_clipboard(code);
        self.copied_code = Some((opener, std::time::Instant::now()));
        ctx.set_handled();
        ctx.request_render();
        ctx.request_anim_frame();
        true
    }

    pub(super) fn code_header_hover(&mut self, ctx: &mut EventCtx<'_>, point: Point) -> bool {
        let next = self.code_header_at(point);
        if self.hovered_code_copy != next {
            self.hovered_code_copy = next;
            ctx.request_render();
        }
        next.is_some()
    }

    pub(super) fn code_header_leave(&mut self, ctx: &mut EventCtx<'_>) {
        if self.hovered_code_copy.take().is_some() {
            self.pointer_hover = false;
            ctx.request_render();
        }
    }

    pub(super) fn code_header_tick(&mut self, ctx: &mut UpdateCtx<'_>) {
        if let Some((_, started)) = self.copied_code {
            if started.elapsed() >= COPIED_FEEDBACK {
                self.copied_code = None;
                ctx.request_render();
            } else {
                crate::ui::timer::wake(ctx.widget_id(), started + COPIED_FEEDBACK);
            }
        }
    }

    /// The existing fence opener already displays the language. Treat that
    /// line as a restrained header with a right-side Copy action.
    pub(super) fn paint_code_header(&self, painter: &mut Painter<'_>, index: usize) {
        let Some(button) = self.code_header_button(index) else { return };
        let theme = theme::current();
        let x0 = self.shift().x + 12.0;
        let x1 = self.shift().x + self.inner_width() - 12.0;
        let divider = self.doc.bottom(index) - 1.0;
        if x1 > x0 && divider > self.doc.top(index) + 2.0 {
            painter.fill(Rect::new(x0, divider, x1, divider + 0.75), theme.block_border.multiply_alpha(0.65)).draw();
        }
        let opener = self.doc.paragraph(index).start;
        let hovered = self.hovered_code_copy == Some(opener);
        let copied = self.copied_code.is_some_and(|(at, start)| at == opener && start.elapsed() < COPIED_FEEDBACK);
        if hovered || copied {
            painter.fill(RoundedRect::from_rect(button, size::ROUNDED_LG), if copied { theme.icon_button_bg_active } else { theme.icon_button_bg_hover }).draw();
        }
        let ink = if hovered || copied { theme.icon_button_icon_hover } else { theme.icon_muted };
        let glyph = if copied { icons::CHECK } else { icons::COPY };
        glyph.draw(painter, button.center() - Vec2::new(COPY_GLYPH / 2.0, COPY_GLYPH / 2.0), COPY_GLYPH, 1.6, ink);
    }

    #[cfg(test)]
    pub(in crate::ui::editor) fn code_header_button_for_test(&self, opener: usize) -> Option<Rect> {
        let index = self.doc.at_offset(opener);
        (self.doc.paragraph(index).start == opener).then(|| self.code_header_button(index)).flatten()
    }

    #[cfg(test)]
    pub(in crate::ui::editor) fn copied_code_for_test(&self) -> Option<usize> {
        self.copied_code.map(|(opener, _)| opener)
    }
}
