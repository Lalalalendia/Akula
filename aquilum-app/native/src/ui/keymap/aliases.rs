use super::chord::Chord;
use super::table::Binding;

impl Binding {
    // Accept macOS hardware-key aliases without crowding the visible shortcut labels.
    pub const fn with_hidden(self, hidden: &'static [Chord]) -> Self {
        Self { hidden, ..self }
    }
}
