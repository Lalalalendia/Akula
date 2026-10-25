use masonry::core::WidgetId;
use crate::ui::editor::TextEditor;
use crate::ui::handle::Handle;
use super::schema::{Action, Number, Setter};

pub enum Binding {
    Switch(Setter),
    Choice(Setter),
    Dropdown { setter: Setter, options: Vec<String>, selected: usize },
    Number { setter: Setter, number: Number, minus: Option<WidgetId>, plus: Option<WidgetId> },
    Step { field: WidgetId, direction: f64 },
    Text { setter: Setter, commit: bool, value: String },
    Color(Setter),
    Commit(Handle<TextEditor>),
    #[cfg(feature = "notion-offline-preview")] BasesDraft, Button(Action),
}
