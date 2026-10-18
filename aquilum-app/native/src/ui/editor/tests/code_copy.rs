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
