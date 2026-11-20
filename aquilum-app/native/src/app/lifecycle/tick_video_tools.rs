use super::*;

impl App {
    pub(super) fn poll_optional_video_tool(&mut self, root: &mut RenderRoot) {
        let revision = (crate::video::optional_ytdlp::revision(), crate::video::optional_ytdlp::quickjs::revision());
        if revision == self.video_tool_revision { return; }
        self.video_tool_revision = revision;
        if matches!(&self.popup, Some(Popup::Settings { dialog, .. })
            if dialog.section == crate::ui::settings::schema::Section::System)
        {
            self.refresh_settings(root);
        }
    }
}
