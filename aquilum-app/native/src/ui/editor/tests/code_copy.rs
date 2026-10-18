use std::sync::{Arc, Mutex};

use super::support::{editor_id, right_click, selected, text, unfocused, TAG};
use crate::ui::editor::resolve::{Book, Fetch, Grid, Media, Request, Resolver};
use crate::ui::editor::TextEditor;

#[derive(Default)]
struct RecordingResolver(Mutex<Vec<Request>>);

impl Resolver for RecordingResolver {
    fn dataview(&self, _query: &str) -> (Fetch<Grid>, u64) {
        (Fetch::Pending, 0)
    }

    fn media(&self, _source: &str, _wiki: bool) -> (Fetch<Media>, u64) {
        (Fetch::Pending, 0)
    }

    fn book(&self, _target: &str) -> (Fetch<Book>, u64) {
        (Fetch::Pending, 0)
    }

    fn request(&self, request: Request) {
        self.0.lock().expect("recording requests").push(request);
    }
}

#[test]
fn right_click_uses_clicked_fence_not_unrelated_selection_or_caret() {
    let source = "до\n~~~rust\nкод 🐱\n~~~\nпосле";
    let recorder = Arc::new(RecordingResolver::default());
    let resolver: Arc<dyn Resolver> = recorder.clone();
    let mut root = unfocused(TextEditor::new(source).markdown().with_resolver(Some(resolver)), 600, 400);
    let id = editor_id(&root);
    root.edit_widget(id, |mut widget| TextEditor::select_byte_range(&mut widget.downcast(), 0, "до".len()));
    let _ = root.redraw();

    let (in_code, in_prose) = {
        let widget = root.get_widget_with_tag(TAG).expect("editor");
        let editor = widget.inner();
        let code = editor.top_of(source.find("код 🐱").unwrap()).expect("code laid out") + 7.0;
        let prose = editor.top_of(source.find("после").unwrap()).expect("prose laid out") + 7.0;
        (code, prose)
    };
    right_click(&mut root, 80.0, in_code);
    right_click(&mut root, 80.0, in_prose);

    let requests: Vec<Request> = recorder.0.lock().expect("request log").drain(..)
        .filter(|r| matches!(r, Request::ContextMenu { .. }))
        .collect();
    assert_eq!(requests.len(), 2, "{requests:?}");
    let Request::ContextMenu { has_selection, code, .. } = &requests[0] else { unreachable!() };
    assert!(*has_selection, "selection outside the code must not be cleared");
    assert_eq!(*code, Some((id, source.find("~~~rust").unwrap())));
    let Request::ContextMenu { code, .. } = &requests[1] else { unreachable!() };
    assert_eq!(*code, None, "plain prose must not offer Copy code");
    assert_eq!(selected(&root), "до");
    assert_eq!(text(&root), source, "opening copy menus must not alter source");
}


#[test]
fn visible_header_copy_buttons_target_the_correct_fence_without_edits() {
    use super::support::{click, editor_id, focused};

    let source = "начало\n~~~rust\nlet a = 1;\n~~~\nмежду\n\u{60}\u{60}\u{60}python\nprint('🐱')\n\u{60}\u{60}\u{60}\nконец";
    let first = source.find("~~~rust").unwrap();
    let second = source.find("\u{60}\u{60}\u{60}python").unwrap();
    let mut root = focused(TextEditor::new(source).markdown(), 640, 480);
    let id = editor_id(&root);
    root.edit_widget(id, |mut widget| TextEditor::select_byte_range(&mut widget.downcast(), 0, "начало".len()));
    let _ = root.redraw();

    let (first_button, second_button) = {
        let widget = root.get_widget_with_tag(TAG).unwrap();
        let editor = widget.inner();
        (
            editor.code_header_button_for_test(first).expect("first code header"),
            editor.code_header_button_for_test(second).expect("second code header"),
        )
    };
    assert!(first_button.y1 < second_button.y0);
    click(&mut root, first_button.center().x, first_button.center().y);
    assert_eq!(
        root.get_widget_with_tag(TAG).unwrap().inner().copied_code_for_test(),
        Some(first),
        "first click must copy the first fenced body"
    );
    click(&mut root, second_button.center().x, second_button.center().y);
    assert_eq!(
        root.get_widget_with_tag(TAG).unwrap().inner().copied_code_for_test(),
        Some(second),
        "second click must target the second body"
    );
    assert_eq!(selected(&root), "начало", "copy button must not move the caret or selection");
    assert_eq!(text(&root), source, "copy button must not edit Markdown");
}

#[test]
fn copy_header_is_not_offered_for_prose_math_or_occluded_long_info() {
    let source = format!(
        "обычный текст\n\u{60}\u{60}\u{60}{}\nbody\n\u{60}\u{60}\u{60}\n$$\nx^2\n$$",
        "language".repeat(60)
    );
    let root = unfocused(TextEditor::new(&source).markdown(), 340, 380);
    let editor = root.get_widget_with_tag(TAG).unwrap();
    let editor = editor.inner();
    assert_eq!(editor.code_header_button_for_test(0), None);
    assert_eq!(editor.code_header_button_for_test(source.find("\u{60}\u{60}\u{60}").unwrap()), None);
    assert_eq!(editor.code_header_button_for_test(source.find("$$").unwrap()), None);
}


#[test]
fn code_header_click_changes_only_its_control_pixels() {
    use super::support::click;

    const W: u16 = 600;
    const H: u16 = 260;
    fn render(root: &mut masonry::app::RenderRoot) -> Vec<u32> {
        let (layers, _) = root.redraw();
        let mut renderer = crate::render::Renderer::new(2);
        renderer.render(&layers, W, H, 1.0, crate::ui::theme::background()).expect("native frame");
        renderer.frame_pixels().to_vec()
    }

    let source = "\u{60}\u{60}\u{60}rust\nfn main() { println!(\"hello\"); }\n\u{60}\u{60}\u{60}";
    let mut root = unfocused(TextEditor::new(source).markdown(), u32::from(W), u32::from(H));
    let button = root.get_widget_with_tag(TAG).unwrap().inner().code_header_button_for_test(0).expect("header button");
    let old = render(&mut root);
    click(&mut root, button.center().x, button.center().y);
    let new = render(&mut root);
    assert_eq!(old.len(), new.len());
    let (mut inside, mut outside) = (0usize, 0usize);
    for (index, (old, new)) in old.iter().zip(new.iter()).enumerate() {
        if old == new {
            continue;
        }
        let x = (index % usize::from(W)) as f64;
        let y = (index / usize::from(W)) as f64;
        if x >= button.x0 - 2.0 && x <= button.x1 + 2.0 && y >= button.y0 - 2.0 && y <= button.y1 + 2.0 {
            inside += 1;
        } else {
            outside += 1;
        }
    }
    assert!(inside >= 8, "copy/check feedback must actually be rendered: {inside} changed pixels");
    assert_eq!(outside, 0, "copy must not repaint different text or surrounding pixels");
    assert_eq!(text(&root), source);
}
