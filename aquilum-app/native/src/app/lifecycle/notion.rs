//! Non-blocking native result delivery for the selected Notion root.
use super::super::*;

impl App {
    pub(super) fn poll_notion(&mut self, root: &mut RenderRoot) {
        let (sync_changed, refreshed) = self.notion_sync.poll();
        if let Some(root_id) = refreshed {
            self.notion_session.open(self.workspace.data_dir.clone(), root_id);
        }
        if (sync_changed || self.notion_session.poll())
            && matches!(&self.popup, Some(Popup::Settings { dialog, .. })
                if dialog.section == crate::ui::settings::schema::Section::Sync)
        {
            self.refresh_settings(root);
        }
    }
}
