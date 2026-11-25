//! Real mounted widget events, including the AccessKit activation path.
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use masonry::accesskit::{Action, ActionRequest, Role, TreeId};
use masonry::app::{RenderRoot, RenderRootOptions, RenderRootSignal, WindowSizePolicy};
use masonry::core::{
    NewWidget, PointerButton, PointerButtonEvent, PointerEvent, PointerId, PointerInfo,
    PointerState, PointerType, PointerUpdate, Widget, WindowEvent,
};
use masonry::dpi::{PhysicalPosition, PhysicalSize};

use crate::ui::icons;
use crate::ui::theme;
use crate::ui::widgets::{Chip, IconButton, ItemButton, LinkButton, TextButton, Variant};

const LABEL: &str = "Reuse event fixture";

pub(in crate::ui) fn check_button(widget: NewWidget<impl Widget + ?Sized>, label: &str, size: PhysicalSize<u32>) {
    let _theme = theme::test_lock();
    let signals: Rc<RefCell<Vec<RenderRootSignal>>> = Rc::default();
    let sink = signals.clone();
    let options = RenderRootOptions {
        default_properties: Arc::new(theme::default_properties()),
        use_system_fonts: true,
        size_policy: WindowSizePolicy::User,
        size,
        scale_factor: 1.0,
        test_font: None,
    };
    let mut root = RenderRoot::new(widget, move |signal| sink.borrow_mut().push(signal), options);
    theme::register_fonts(&mut root);
    root.handle_window_event(WindowEvent::EnableAccessTree);
    let (_, update) = root.redraw();
    let tree = update.expect("button must publish an accessibility tree");
    let (id, node) = tree.nodes.iter().find(|(_, node)| {
        node.role() == Role::Button && node.label() == Some(label)
    }).expect("button must keep its own accessible name and role");
    assert!(node.supports_action(Action::Click));
    let actions = || {
        signals.borrow().iter().filter(|s| {
            matches!(s, RenderRootSignal::Action(_, widget_id) if masonry::accesskit::NodeId::from(*widget_id) == *id)
        }).count()
    };
    let mouse = PointerInfo {
        pointer_id: Some(PointerId::PRIMARY),
        persistent_device_id: None,
        pointer_type: PointerType::Mouse,
    };
    let mut state = PointerState {
        position: PhysicalPosition::new(12.0, 12.0),
        count: 1,
        scale_factor: 1.0,
        ..PointerState::default()
    };
    let move_pointer = |root: &mut RenderRoot, state: &PointerState| {
        root.handle_pointer_event(PointerEvent::Move(PointerUpdate {
            pointer: mouse, current: state.clone(), coalesced: vec![], predicted: vec![],
        }));
        let _ = root.redraw();
    };

    move_pointer(&mut root, &state);
    state.buttons.insert(PointerButton::Primary);
    root.handle_pointer_event(PointerEvent::Down(PointerButtonEvent {
        pointer: mouse, button: Some(PointerButton::Primary), state: state.clone(),
    }));
    assert_eq!(actions(), 0, "press alone must not activate a button");
    state.buttons.remove(PointerButton::Primary);
    root.handle_pointer_event(PointerEvent::Up(PointerButtonEvent {
        pointer: mouse, button: Some(PointerButton::Primary), state: state.clone(),
    }));
    let _ = root.redraw();
    assert_eq!(actions(), 1, "one complete click must emit one action");

    root.handle_pointer_event(PointerEvent::Up(PointerButtonEvent {
        pointer: mouse, button: Some(PointerButton::Primary), state: state.clone(),
    }));
    let _ = root.redraw();
    assert_eq!(actions(), 1, "unpaired release must not activate");

    root.handle_access_event(ActionRequest {
        target_tree: TreeId::ROOT,
        target_node: *id,
        action: Action::Click,
        data: None,
    });
    let _ = root.redraw();
    assert_eq!(actions(), 2, "accessible Click must emit exactly one action");
}

#[test]
fn item_button_click_and_accessibility_contract() {
    check_button(NewWidget::new(ItemButton::new(icons::X, LABEL)).erased(), LABEL, PhysicalSize::new(400, 60));
}

#[test]
fn link_button_click_and_accessibility_contract() {
    check_button(LinkButton::new(LABEL).erased(), LABEL, PhysicalSize::new(400, 60));
}

#[test]
fn text_button_click_and_accessibility_contract() {
    check_button(NewWidget::new(TextButton::new(LABEL, Variant::Primary)).erased(), LABEL, PhysicalSize::new(400, 60));
}

#[test]
fn chip_click_and_accessibility_contract() {
    check_button(Chip::new(LABEL, true).erased(), LABEL, PhysicalSize::new(400, 60));
}

#[test]
fn icon_button_click_and_accessibility_contract() {
    check_button(NewWidget::new(IconButton::new(icons::X, LABEL)).erased(), LABEL, PhysicalSize::new(400, 60));
}
