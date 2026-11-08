use masonry::app::RenderRoot;
use super::App;
use crate::ui::editor::TextEditor;

impl App {
pub(crate) fn paste_formatted(
        &mut self,
        root: &mut RenderRoot,
        clipboard: &mut arboard::Clipboard,
    ) -> bool {
        let Some(ids) = self.note_ids.as_ref()
            .filter(|ids| root.focused_widget() == Some(ids.body.id()))
        else {
            return false;
        };
        if !self.workspace.core.settings.get_config().editor.paste_formatting {
            return false;
        }
        let Some(markdown) = clipboard.get().html().ok()
            .and_then(|html| aquilum_core::markdown::html::to_markdown(&html))
        else {
            return false;
        };
        ids.body.edit(root, |mut editor| TextEditor::paste_markdown(&mut editor, &markdown))
            .unwrap_or(false)
    }
}
