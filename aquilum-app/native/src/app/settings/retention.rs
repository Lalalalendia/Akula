use super::*;

impl App {
    pub(in crate::app) fn cleanup_retention(&self) {
        let Some(workspace) = self.tree.root().map(Path::to_path_buf) else { return };
        let core = std::sync::Arc::clone(&self.workspace.core);
        let (done, wake) = (std::sync::Arc::clone(&self.retention_done), std::sync::Arc::clone(&self.workspace.wake));
        std::thread::spawn(move || {
            let config = core.settings.get_config();
            if let Err(error) = gate::cleanup_retention(&core, &workspace, config.trash.retention_days) {
                report!("корзина не очищена: {error:?}");
            }
            if let Err(error) = core.history.maintain_retention(&workspace, config.history.retention_days) {
                report!("обслуживание истории остановлено: {error}");
            }
            if let Err(error) = core.ui_state.cleanup(config.trash.retention_days, crate::dates::now_ms()) {
                report!("состояние документов не очищено: {error:?}");
            }
            done.store(true, std::sync::atomic::Ordering::Release);
            wake();
        });
    }

    pub(in crate::app) fn on_retention_done(&mut self, root: &mut RenderRoot) {
        if self.retention_done.swap(false, std::sync::atomic::Ordering::Acquire)
            && matches!(&self.popup, Some(Popup::Settings { dialog, .. }) if matches!(dialog.section, Section::Trash | Section::History))
        {
            self.refresh_settings(root);
        }
    }
}
