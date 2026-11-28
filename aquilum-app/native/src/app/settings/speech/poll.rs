use super::*;

impl App {
    pub(in crate::app) fn poll_speech_settings(&mut self, root: &mut RenderRoot) {
        self.poll_transcript(root);
        if let Some(track) = self
            .workspace.speech
            .get()
            .and_then(|dispatcher| dispatcher.take_caption_update())
        {
            crate::video::captions::select(track);
            if let Some(id) = self.editor_id() {
                root.edit_widget(id, |mut editor| editor.ctx.request_post_paint());
            }
        }
        if matches!(&self.popup, Some(Popup::Settings { dialog, menu: None }) if dialog.section == Section::System)
            && self
                .workspace
                .speech
                .get()
                .is_some_and(|dispatcher| dispatcher.updated())
        {
            self.refresh_settings(root);
        }
    }
}
