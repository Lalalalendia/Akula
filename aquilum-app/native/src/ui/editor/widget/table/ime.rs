//! Route IME composition to the active table cell rather than the document caret.
use masonry::core::Ime;

use super::*;

impl TextEditor {
    pub(in super::super) fn table_ime(&mut self, ctx: &mut EventCtx<'_>, ime: &Ime) -> bool {
        if !self.table.as_ref().is_some_and(|s| s.active.is_some()) {
            return false;
        }
        let Some((session, view)) = self.table_parts() else { return false };
        let outcome = match ime {
            Ime::Preedit(text, cursor) => session.preedit(view, text, *cursor),
            Ime::Commit(text) => session.insert(view, text),
            Ime::Enabled | Ime::Disabled => {
                session.compose = None;
                Outcome::None
            }
        };
        self.apply(ctx, outcome);
        ctx.set_handled();
        true
    }

    pub(in super::super) fn table_text(&mut self, ctx: &mut EventCtx<'_>, text: &str) -> bool {
        if !self.table.as_ref().is_some_and(|s| s.active.is_some()) {
            return false;
        }
        let Some((session, view)) = self.table_parts() else { return false };
        let outcome = session.insert(view, text);
        self.apply(ctx, outcome);
        ctx.set_handled();
        true
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use masonry::app::{RenderRoot, RenderRootOptions, WindowSizePolicy};
    use masonry::core::{Ime, NewWidget, PointerButton, PointerButtonEvent, PointerEvent, PointerId, PointerInfo, PointerState, PointerType, PointerUpdate, TextEvent, WidgetTag};
    use masonry::dpi::{PhysicalPosition, PhysicalSize};
    use masonry::theme::default_property_set;

    use super::TextEditor;

    const TAG: WidgetTag<TextEditor> = WidgetTag::named("table ime test");

    fn root(source: &str) -> RenderRoot {
        let options = RenderRootOptions {
            default_properties: Arc::new(default_property_set()),
            use_system_fonts: true,
            size_policy: WindowSizePolicy::User,
            size: PhysicalSize::new(600, 600),
            scale_factor: 1.0,
            test_font: None,
        };
        let mut root = RenderRoot::new(
            NewWidget::new(TextEditor::new(source).markdown()).with_tag(TAG).erased(),
            |_| {},
            options,
        );
        root.focus_on(Some(root.get_widget_with_tag(TAG).expect("editor").id()));
        let _ = root.redraw();
        root
    }

    fn click(root: &mut RenderRoot) {
        let pointer = PointerInfo {
            pointer_id: Some(PointerId::PRIMARY),
            persistent_device_id: None,
            pointer_type: PointerType::Mouse,
        };
        let mut state = PointerState {
            position: PhysicalPosition::new(60.0, 50.0),
            count: 1,
            scale_factor: 1.0,
            ..PointerState::default()
        };
        root.handle_pointer_event(PointerEvent::Move(PointerUpdate {
            pointer: pointer.clone(), current: state.clone(), coalesced: vec![], predicted: vec![],
        }));
        state.buttons.insert(PointerButton::Primary);
        root.handle_pointer_event(PointerEvent::Down(PointerButtonEvent {
            pointer: pointer.clone(), button: Some(PointerButton::Primary), state: state.clone(),
        }));
        state.buttons.remove(PointerButton::Primary);
        root.handle_pointer_event(PointerEvent::Up(PointerButtonEvent {
            pointer, button: Some(PointerButton::Primary), state,
        }));
        let _ = root.redraw();
    }

    fn text(root: &RenderRoot) -> String {
        root.get_widget_with_tag(TAG).expect("editor").inner().text()
    }

    #[test]
    fn composition_replaces_preedit_inside_clicked_cell_not_document_body() {
        let source = "| a | b |\n| --- | --- |\n| 1 | 2 |\nпосле";
        let mut root = root(source);
        click(&mut root);
        for (value, at) in [("こ", 3), ("候補", 6)] {
            root.handle_text_event(TextEvent::Ime(Ime::Preedit(value.into(), Some((at, at)))));
            assert_eq!(text(&root), format!("| a{value} | b |\n| --- | --- |\n| 1 | 2 |\nпосле"));
        }
        root.handle_text_event(TextEvent::Ime(Ime::Commit("漢字".into())));
        root.handle_text_event(TextEvent::Ime(Ime::Commit("!".into())));
        assert_eq!(text(&root), "| a漢字! | b |\n| --- | --- |\n| 1 | 2 |\nпосле");
    }
}
