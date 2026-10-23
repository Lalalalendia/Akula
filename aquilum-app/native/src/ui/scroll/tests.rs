use std::sync::Arc;
use std::time::Duration;

use masonry::app::{RenderRoot, RenderRootOptions, WindowSizePolicy};
use masonry::core::keyboard::Modifiers;
use masonry::core::{NewWidget, PointerEvent, PointerId, PointerInfo, PointerScrollEvent, PointerState, PointerType, ScrollDelta, WidgetTag, WindowEvent};
use masonry::dpi::{PhysicalPosition, PhysicalSize};
use masonry::layout::Length;
use masonry::theme::default_property_set;
use masonry::widgets::SizedBox;

use super::ScrollArea;

const TAG: WidgetTag<ScrollArea<SizedBox>> = WidgetTag::named("прокрутка");
const MOUSE: PointerInfo = PointerInfo { pointer_id: Some(PointerId::PRIMARY), persistent_device_id: None, pointer_type: PointerType::Mouse };

fn root() -> RenderRoot {
    let content = NewWidget::new(SizedBox::empty().height(Length::px(5000.0)));
    let options = RenderRootOptions {
        default_properties: Arc::new(default_property_set()),
        use_system_fonts: false,
        size_policy: WindowSizePolicy::User,
        size: PhysicalSize::new(400, 300),
        scale_factor: 1.0,
        test_font: None,
    };
    let mut root = RenderRoot::new(NewWidget::new(ScrollArea::new(content)).with_tag(TAG).erased(), |_| {}, options);
    let _ = root.redraw();
    root
}

fn offset(root: &RenderRoot) -> f64 {
    root.get_widget_with_tag(TAG).expect("прокрутка").inner().offset
}

fn scroll(root: &mut RenderRoot, delta: ScrollDelta) {
    let state = PointerState { position: PhysicalPosition::new(200.0, 150.0), modifiers: Modifiers::empty(), count: 1, scale_factor: 1.0, ..PointerState::default() };
    root.handle_pointer_event(PointerEvent::Scroll(PointerScrollEvent { pointer: MOUSE, delta, state }));
}

fn frames(root: &mut RenderRoot, n: usize, trace: &mut Vec<f64>) {
    for _ in 0..n {
        root.handle_window_event(WindowEvent::AnimFrame(Duration::from_millis(8)));
        trace.push(offset(root));
    }
}

#[test]
fn touchpad_at_top_never_pushes_page_down() {
    let mut root = root();
    let mut trace = Vec::new();
    for y in [0.4, 0.7, 1.0, 0.2, 2.0, 0.05] {
        scroll(&mut root, ScrollDelta::LineDelta(0.0, y));
        trace.push(offset(&root));
        frames(&mut root, 3, &mut trace);
    }
    assert!(trace.iter().all(|&o| o == 0.0), "смещение у верхнего края: {trace:?}");
}

#[test]
fn reversing_direction_does_not_overshoot() {
    let mut root = root();
    let mut trace = Vec::new();
    for _ in 0..5 {
        scroll(&mut root, ScrollDelta::LineDelta(0.0, -1.0));
        frames(&mut root, 2, &mut trace);
    }
    let deepest = offset(&root);
    for y in [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0] {
        scroll(&mut root, ScrollDelta::LineDelta(0.0, y));
        frames(&mut root, 2, &mut trace);
    }
    frames(&mut root, 40, &mut trace);
    assert_eq!(offset(&root), 0.0);
    let after: Vec<f64> = trace.iter().copied().skip(10).collect();
    assert!(after.iter().all(|&o| o <= deepest), "после разворота страницу унесло ниже: {after:?}");
    assert!(after.windows(2).all(|w| w[1] <= w[0]), "движение вверх с рывками назад: {after:?}");
}

#[test]
fn wheel_glide_lands_exactly_and_stays_in_bounds() {
    let mut root = root();
    let mut trace = Vec::new();
    scroll(&mut root, ScrollDelta::LineDelta(0.0, -3.0));
    frames(&mut root, 40, &mut trace);
    assert_eq!(offset(&root), 3.0 * super::LINE);
    assert!(trace.windows(2).all(|w| w[1] >= w[0]), "доведение колесом пошло назад: {trace:?}");
    scroll(&mut root, ScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 100_000.0)));
    assert_eq!(offset(&root), 0.0);
    scroll(&mut root, ScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -100_000.0)));
    assert_eq!(offset(&root), 5000.0 - 300.0);
}

#[test]
fn continuous_wheel_moves_every_frame_without_jolts() {
    let mut root = root();
    let mut trace = vec![offset(&root)];
    for _ in 0..10 {
        scroll(&mut root, ScrollDelta::LineDelta(0.0, -1.0));
        std::thread::sleep(Duration::from_millis(64));
        frames(&mut root, 8, &mut trace);
    }
    let steps: Vec<f64> = trace.windows(2).map(|w| w[1] - w[0]).skip(16).collect();
    assert!(steps.iter().all(|&s| s > 0.0), "кадр без движения при непрерывном вращении: {steps:?}");
    assert!(steps.windows(2).all(|w| w[1] / w[0] < 2.0 && w[0] / w[1] < 2.0), "скачок скорости между кадрами: {steps:?}");
}

#[test]
fn nested_horizontal_wheel_normalizes_all_delta_units() {
    use masonry::core::ScrollDelta;
    use masonry::dpi::PhysicalPosition;
    let pixel = super::pixels(&ScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 80.0)), 2.0, 600.0);
    assert_eq!(pixel.y, 40.0);
    assert_eq!(super::sideways(pixel, true), 40.0);
    assert_eq!(super::sideways(pixel, false), 0.0);
    let lines = super::pixels(&ScrollDelta::LineDelta(2.0, 1.0), 2.0, 600.0);
    assert_eq!(lines.x, 2.0 * super::LINE);
    assert_eq!(super::sideways(lines, false), lines.x);
    let page = super::pixels(&ScrollDelta::PageDelta(0.0, 1.0), 1.0, 600.0);
    assert_eq!(super::sideways(page, true), 600.0);
    assert_eq!(super::sideways(page, false), 0.0);
}

#[test]
fn invalid_geometry_does_not_create_nonfinite_horizontal_motion() {
    use masonry::core::ScrollDelta;
    use masonry::dpi::PhysicalPosition;
    assert_eq!(super::pixels(&ScrollDelta::PixelDelta(PhysicalPosition::new(50.0, 100.0)), 0.0, 600.0).x, 0.0);
    assert_eq!(super::pixels(&ScrollDelta::PageDelta(1.0, 1.0), 1.0, f64::NAN).x, 0.0);
    assert_eq!(super::sideways(super::pixels(&ScrollDelta::LineDelta(0.0, 1.0), 1.0, 500.0), true), super::LINE);
}

fn horizontal_root() -> RenderRoot {
    let content = NewWidget::new(SizedBox::empty().width(Length::px(5_000.0)));
    let options = RenderRootOptions {
        default_properties: Arc::new(default_property_set()),
        use_system_fonts: false,
        size_policy: WindowSizePolicy::User,
        size: PhysicalSize::new(400, 300),
        scale_factor: 1.0,
        test_font: None,
    };
    let mut root = RenderRoot::new(
        NewWidget::new(ScrollArea::horizontal(content)).with_tag(TAG).erased(),
        |_| {},
        options,
    );
    let _ = root.redraw();
    root
}

#[test]
fn horizontal_page_wheel_is_handled_by_scroll_container() {
    let mut root = horizontal_root();
    let mut trace = Vec::new();
    // A horizontal page command scrolls by the viewport, not by LINE pixels.
    scroll(&mut root, ScrollDelta::PageDelta(-1.0, 0.0));
    frames(&mut root, 40, &mut trace);
    assert!((offset(&root) - 400.0).abs() < 0.01,
        "horizontal page event did not advance by viewport: {trace:?}");
    // Precision touchpad remains immediate; large deltas must clamp safely.
    scroll(&mut root, ScrollDelta::PixelDelta(PhysicalPosition::new(-100_000.0, 0.0)));
    assert!((offset(&root) - (5_000.0 - 400.0)).abs() < 0.01);
    scroll(&mut root, ScrollDelta::PixelDelta(PhysicalPosition::new(100_000.0, 0.0)));
    assert_eq!(offset(&root), 0.0);
}

#[test]
fn nonfinite_scroll_input_is_rejected_before_reaching_nested_offsets() {
    use masonry::core::ScrollDelta;
    use masonry::dpi::PhysicalPosition;
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let pixel = super::pixels(&ScrollDelta::PixelDelta(PhysicalPosition::new(value, 40.0)), 1.0, 600.0);
        assert_eq!(pixel, masonry::kurbo::Vec2::ZERO);
        let page = super::pixels(&ScrollDelta::PageDelta(1.0, value as f32), 1.0, 600.0);
        assert_eq!(page, masonry::kurbo::Vec2::ZERO);
        let line = super::pixels(&ScrollDelta::LineDelta(value as f32, 1.0), 1.0, 600.0);
        assert_eq!(line, masonry::kurbo::Vec2::ZERO);
        assert_eq!(super::sideways(masonry::kurbo::Vec2::new(value, 0.0), false), 0.0);
    }
}

#[test]
fn find_marker_rows_are_deduplicated_and_scale_aware() {
    let ticks = super::find_marks::marker_rows(&[0.0, 0.25, 0.251, 1.0, f64::NAN], 100.0, 1.0);
    assert_eq!(ticks, [0, 25, 99]);
    assert_eq!(super::find_marks::marker_rows(&[0.0, 1.0], 100.0, 2.0), [0, 199]);
    assert!(super::find_marks::marker_rows(&[0.5], 0.0, 1.0).is_empty());
    assert!(super::find_marks::marker_rows(&[0.5], 100.0, f64::NAN).is_empty());
}

#[test]
fn find_marker_rows_are_bounded_even_for_huge_documents() {
    let marks = super::find_marks::marker_rows(&[0.0, 0.5, 1.0, f64::INFINITY, f64::NEG_INFINITY], 1e12, 3.0);
    assert_eq!(marks, [0, 32768, 65535]);
    assert!(super::find_marks::marker_rows(&[], 1e12, 3.0).is_empty());
}
