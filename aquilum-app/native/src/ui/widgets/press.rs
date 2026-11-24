use masonry::accesskit::Action;
use masonry::core::{
    AccessEvent, EventCtx, PointerButton, PointerButtonEvent, PointerEvent, Update, UpdateCtx,
};

use crate::ui::text;

/// Нажатие кнопки.
#[derive(Debug)]
pub struct Pressed;

/// Общая логика нажатия: захват указателя, действие при отпускании над кнопкой.
#[derive(Default)]
pub struct Press {
    pub down: bool,
}

impl Press {
    pub fn pointer(&mut self, ctx: &mut EventCtx<'_>, event: &PointerEvent) -> bool {
        self.click(ctx, event).is_some()
    }

    pub fn click<'e>(&mut self, ctx: &mut EventCtx<'_>, event: &'e PointerEvent) -> Option<&'e PointerButtonEvent> {
        match event {
            PointerEvent::Down(e) if e.button == Some(PointerButton::Primary) => {
                self.down = true;
                ctx.capture_pointer();
                ctx.request_paint_only();
                None
            }
            PointerEvent::Up(e) if e.button == Some(PointerButton::Primary) && self.down => {
                self.down = false;
                ctx.request_paint_only();
                ctx.is_hovered().then_some(e)
            }
            PointerEvent::Cancel(_) => {
                self.down = false;
                None
            }
            _ => None,
        }
    }
}

// Shared by ItemButton, LinkButton, TextButton and Chip. The action and
// invalidation contracts stay identical; each widget owns its appearance.
pub(super) fn submit_pointer(press: &mut Press, ctx: &mut EventCtx<'_>, event: &PointerEvent) {
    if press.pointer(ctx, event) {
        ctx.submit_action::<Pressed>(Pressed);
    }
}

pub(super) fn submit_access(ctx: &mut EventCtx<'_>, event: &AccessEvent) {
    if event.action == Action::Click {
        ctx.submit_action::<Pressed>(Pressed);
    }
}

pub(super) fn update_button_line(line: &mut text::Line, ctx: &mut UpdateCtx<'_>, event: &Update) {
    match event {
        Update::HoveredChanged(_) => ctx.request_paint_only(),
        Update::FontsChanged => {
            line.clear();
            ctx.request_layout();
        }
        _ => {}
    }
}

#[cfg(test)]
#[path = "press_tests.rs"]
mod tests;
