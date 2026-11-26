//! Widget event contract, without sharing widget state, layout or appearance.
use masonry::accesskit::Action;
use masonry::core::{AccessEvent, EventCtx, PointerEvent, Update, UpdateCtx};
use crate::ui::text;
use super::{Press, Pressed};

// Shared by ItemButton, LinkButton, TextButton and Chip. The action and
// invalidation contracts stay identical; each widget owns its appearance.
pub(in crate::ui) fn submit_pointer(press: &mut Press, ctx: &mut EventCtx<'_>, event: &PointerEvent) {
    if press.pointer(ctx, event) {
        ctx.submit_action::<Pressed>(Pressed);
    }
}

pub(in crate::ui) fn submit_access(ctx: &mut EventCtx<'_>, event: &AccessEvent) {
    if event.action == Action::Click {
        ctx.submit_action::<Pressed>(Pressed);
    }
}

pub(in crate::ui) fn update_button_line(line: &mut text::Line, ctx: &mut UpdateCtx<'_>, event: &Update) {
    match event {
        Update::HoveredChanged(_) => ctx.request_paint_only(),
        Update::FontsChanged => {
            line.clear();
            ctx.request_layout();
        }
        _ => {}
    }
}

// Emit only the exact same Widget event methods; the optional line field
// opts into font-cache invalidation. Widgets with custom actions stay separate.
macro_rules! press_events {
    ($($line:ident)?) => {
        fn on_pointer_event(&mut self, ctx: &mut masonry::core::EventCtx<'_>, _props: &mut masonry::core::PropertiesMut<'_>, event: &masonry::core::PointerEvent) {
            $crate::ui::widgets::press::submit_pointer(&mut self.press, ctx, event);
        }
        fn on_access_event(&mut self, ctx: &mut masonry::core::EventCtx<'_>, _props: &mut masonry::core::PropertiesMut<'_>, event: &masonry::core::AccessEvent) {
            $crate::ui::widgets::press::submit_access(ctx, event);
        }
        $(
            fn update(&mut self, ctx: &mut masonry::core::UpdateCtx<'_>, _props: &mut masonry::core::PropertiesMut<'_>, event: &masonry::core::Update) {
                $crate::ui::widgets::press::update_button_line(&mut self.$line, ctx, event);
            }
        )?
    };
}
pub(in crate::ui) use press_events;
