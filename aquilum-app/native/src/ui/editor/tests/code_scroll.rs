use super::support::{unfocused, TAG};
use crate::ui::editor::TextEditor;

#[test]
fn code_line_has_an_explicit_narrow_viewport_even_when_layout_overflows() {
    let long = format!("value_{}🐱", "x".repeat(500));
    let source = format!("before\n~~~rust\n{long}\n~~~\nafter");
    let root = unfocused(TextEditor::new(&source).markdown(), 220, 400);
    let editor = root.get_widget_with_tag(TAG).expect("editor");
    let editor = editor.inner();
    let (width, lines) = editor.code_line_shape_for_test(2).expect("code body laid out");
    let clip = editor.code_viewport(2).expect("fenced block viewport");
    assert_eq!(lines, 1);
    assert!(width > clip.width() * 4.0);
    assert!(clip.x0 >= 0.0 && clip.x1 <= 220.0, "no bleed past panel");
    assert!(editor.code_viewport(0).is_none(), "prose must not be clipped");
    assert!(editor.code_viewport(4).is_none(), "following prose must not be clipped");
}


#[test]
fn horizontal_wheel_moves_two_fenced_blocks_independently_without_changing_source() {
    use super::support::{wheel, text};

    let first_line = "alpha🐱".repeat(90);
    let second_line = "beta👁".repeat(90);
    let source = format!("before\n~~~rust\n{first_line}\n~~~\nbetween\n~~~python\n{second_line}\n~~~\nafter");
    let first = source.find("~~~rust").unwrap();
    let second = source.find("~~~python").unwrap();
    let mut root = unfocused(TextEditor::new(&source).markdown(), 250, 440);
    let first_y = root.get_widget_with_tag(TAG).unwrap().inner().top_of(first + "~~~rust\n".len()).unwrap() + 7.0;
    let second_y = root.get_widget_with_tag(TAG).unwrap().inner().top_of(second + "~~~python\n".len()).unwrap() + 7.0;

    wheel(&mut root, (95.0, first_y), (-180.0, 0.0));
    let first_after = root.get_widget_with_tag(TAG).unwrap().inner().code_scroll_for_test(first);
    assert!(first_after > 0.0, "horizontal wheel must move the first code block");
    assert_eq!(root.get_widget_with_tag(TAG).unwrap().inner().code_scroll_for_test(second), 0.0);
    wheel(&mut root, (95.0, second_y), (-55.0, 0.0));
    let editor = root.get_widget_with_tag(TAG).unwrap();
    assert_eq!(editor.inner().code_scroll_for_test(first), first_after);
    assert!(editor.inner().code_scroll_for_test(second) > 0.0);
    assert_eq!(text(&root), source, "view motion must not create document edits");
}

#[test]
fn hit_testing_tracks_horizontal_scroll_instead_of_using_unshifted_bytes() {
    use super::support::wheel;
    use masonry::kurbo::{Affine, Point};

    let line = "rusty".repeat(160);
    let source = format!("~~~rust\n{line}\n~~~");
    let mut root = unfocused(TextEditor::new(&source).markdown(), 200, 260);
    let y = root.get_widget_with_tag(TAG).unwrap().inner().top_of("~~~rust\n".len()).unwrap() + 7.0;
    let click = Point::new(82.0, y);
    let before = root.get_widget_with_tag(TAG).unwrap().inner().offset_at(Affine::IDENTITY, click);
    wheel(&mut root, (click.x, click.y), (-180.0, 0.0));
    let after = root.get_widget_with_tag(TAG).unwrap().inner().offset_at(Affine::IDENTITY, click);
    assert!(after > before, "same screen x must hit later source bytes when code is scrolled");
}

#[test]
fn keyboard_caret_follow_reveals_the_end_of_a_long_fence() {
    use super::support::editor_id;
    let line = "longline".repeat(160);
    let source = format!("~~~rust\n{line}\n~~~");
    let opener = 0;
    let end = "~~~rust\n".len() + line.len();
    let mut root = unfocused(TextEditor::new(&source).markdown(), 240, 220);
    let id = editor_id(&root);
    root.edit_widget(id, |mut editor| TextEditor::select_byte_range(&mut editor.downcast(), end, end));
    let _ = root.redraw();
    assert!(root.get_widget_with_tag(TAG).unwrap().inner().code_scroll_for_test(opener) > 0.0,
        "placing caret at the end of a long unwrapped code line must reveal it");
}


#[test]
fn native_pixels_outside_code_viewport_are_unchanged_by_long_code_text() {
    use masonry::app::{RenderRoot, RenderRootOptions, WindowSizePolicy};
    use masonry::core::NewWidget;
    use masonry::dpi::PhysicalSize;
    use std::sync::Arc;

    let _theme_lock = crate::ui::theme::test_lock();
    const W: u16 = 250;
    const H: u16 = 220;
    fn frame(source: &str) -> (Vec<u32>, masonry::kurbo::Rect) {
        let opts = RenderRootOptions {
            default_properties: Arc::new(masonry::theme::default_property_set()),
            use_system_fonts: true,
            size_policy: WindowSizePolicy::User,
            size: PhysicalSize::new(u32::from(W), u32::from(H)),
            scale_factor: 1.0,
            test_font: None,
        };
        let mut root = RenderRoot::new(
            NewWidget::new(TextEditor::new(source).markdown()).with_tag(TAG).erased(),
            |_| {}, opts,
        );
        let (layers, _) = root.redraw();
        let clip = root.get_widget_with_tag(TAG).unwrap().inner()
            .code_viewport(1).expect("code body viewport");
        let mut renderer = crate::render::Renderer::new(2);
        renderer.render(&layers, W, H, 1.0, crate::ui::theme::background())
            .expect("native frame");
        (renderer.frame_pixels().to_vec(), clip)
    }

    let short = "~~~rust\nx\n~~~\nfollowing";
    let long = format!("~~~rust\n{}\n~~~\nfollowing", "W".repeat(500));
    let (short_pixels, before) = frame(short);
    let (long_pixels, after) = frame(&long);
    assert_eq!(before, after, "long lines must not alter block geometry");
    assert_eq!(short_pixels.len(), long_pixels.len());
    let mut changed_inside = 0;
    for y in (after.y0.ceil() as usize + 2)..(after.y1.floor() as usize - 2) {
        for x in 0..usize::from(W) {
            let index = y * usize::from(W) + x;
            if short_pixels[index] == long_pixels[index] {
                continue;
            }
            if (x as f64) < after.x0 - 1.0 || (x as f64) > after.x1 + 1.0 {
                panic!("code glyphs leaked beyond viewport at {x},{y}");
            }
            changed_inside += 1;
        }
    }
    assert!(changed_inside > 8, "control: changed source must alter visible code glyphs");
}
