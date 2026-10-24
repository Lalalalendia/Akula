use super::super::*;

impl App {
    pub fn tick(&mut self, root: &mut RenderRoot, now: Instant) {
        if self.yandex_probe.poll()
            && matches!(&self.popup, Some(Popup::Settings { dialog, .. })
                if dialog.section == crate::ui::settings::schema::Section::Sync)
        {
            self.refresh_settings(root);
        }
        #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
        if self.s3_pair.poll()
            && matches!(&self.popup, Some(Popup::Settings { dialog, .. })
                if dialog.section == crate::ui::settings::schema::Section::Sync)
        {
            self.refresh_settings(root);
        }
        #[cfg(all(windows, feature = "server-readonly-preflight"))]
        self.on_server_probe(root);
        #[cfg(feature = "notion-offline-preview")]
        self.poll_notion(root);
        if self.view_due.is_some_and(|t| t <= now) {
            self.flush_view();
        }
        if let Some(note) = self.note.as_mut()
            && note.tick(&self.workspace.core, &self.text, now)
        {
            self.refresh_save_error(root);
        }
        if let Some(text) = self.note.as_mut().and_then(crate::note::Note::take_replaced) {
            self.set_text(root, text);
            self.refresh_save_error(root);
        }
    }
}
