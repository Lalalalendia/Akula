use super::{LoadState, SelectionView};

impl Default for SelectionView {
    fn default() -> Self {
        Self {
            state: LoadState::Idle, root: None,
            displayed_pages: Vec::new(),
            page_start: 0, page_count: 0, chosen: None,
        }
    }
}
