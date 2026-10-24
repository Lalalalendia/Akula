//! Non-blocking native result delivery for the selected Notion root.
use super::super::*;

impl App {
    pub(super) fn poll_notion(&mut self, root: &mut RenderRoot) {
        if self.notion_session.poll()
            && matches!(&self.popup, Some(Popup::Settings { dialog, .. })
                if dialog.section == crate::ui::settings::schema::Section::Sync)
        {
            self.refresh_settings(root);
        }
    }
}
