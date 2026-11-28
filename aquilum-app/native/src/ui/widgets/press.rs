use masonry::core::{
    EventCtx, PointerButton, PointerButtonEvent, PointerEvent,
};

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

#[path = "press_contract.rs"]
mod contract;
pub(in crate::ui) use contract::{press_events, submit_pointer, submit_access, update_button_line};

#[cfg(test)]
#[path = "press_tests.rs"]
pub(in crate::ui) mod tests;
