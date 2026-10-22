use std::time::Duration;
use masonry::app::RenderRoot;

use super::super::App;
use crate::file_ops;
use crate::frontmatter;
use crate::ui::widgets::Slot;

impl App {
    pub(super) fn body_edited(&mut self, root: &mut RenderRoot, _edits: &[crate::ui::editor::Edit]) {
        let had = frontmatter::end_of(&self.text).is_some();
        // The widget already applied all edits (including multi-cursor changes).
        // Cloning its persistent rope avoids replaying every edit on a second
        // large rope and keeps the persisted text identical to the visible text.
        self.adopt_body(root);
        self.queue_save();
        self.refresh_hero(root);
        self.metadata_changed(root, had);
        self.refresh_find_marks(root);
    }

    fn adopt_body(&mut self, root: &RenderRoot) {
        let Some(mut body) = self.note_ids.as_ref().and_then(|ids| ids.body.get(root)).map(|editor| editor.inner().rope()) else { return };
        if self.hidden > 0 {
            // Preserve hidden frontmatter, which is not part of the editor widget.
            body.insert(0, self.text.byte_slice(..crate::rope::floor(&self.text, self.hidden)).to_string());
        }
        self.text = body;
    }

    pub(in crate::app) fn commit_text(&mut self, root: &mut RenderRoot, text: crop::Rope) {
        let had = frontmatter::end_of(&self.text).is_some();
        self.text = text;
        self.queue_save();
        self.metadata_changed(root, had);
        self.sync_body(root);
        self.refresh_hero(root);
    }

    fn queue_save(&mut self) {
        if let Some(note) = &mut self.note {
            let debounce = self.workspace.core.settings.get_config().editor.save_debounce_ms;
            note.edit(Duration::from_millis(u64::from(debounce)));
        }
    }

    fn metadata_changed(&mut self, root: &mut RenderRoot, had: bool) {
        let has = frontmatter::end_of(&self.text).is_some();
        if has == had {
            return;
        }
        if has && let Some(note) = &self.note {
            self.interface.set_metadata_expanded(&note.path, true);
        }
        let metadata = crate::ui::note::metadata_widget(self.metadata_state());
        if let Some(ids) = &self.note_ids {
            ids.metadata.edit(root, |mut slot| Slot::set_child(&mut slot, metadata));
        }
    }

    pub(super) fn toggle_metadata(&mut self, root: &mut RenderRoot) {
        let Some(expanded) = self.metadata_state() else { return };
        let Some(path) = self.note.as_ref().map(|n| n.path.clone()) else { return };
        self.interface.set_metadata_expanded(&path, !expanded);
        self.sync_body(root);
    }

    pub(super) fn commit_title(&mut self, root: &mut RenderRoot, input: &str) {
        let Some(from) = self.note.as_ref().map(|n| n.path.clone()) else { return };
        match file_ops::title_target(&from, input) {
            Some((to, _)) if self.renaming_to.as_ref() == Some(&to) => {}
            Some((to, _)) => {
                self.renaming_to = Some(to.clone());
                self.rename_file(root, from, to);
            }
            None => self.refresh_note_names(root),
        }
    }
}
