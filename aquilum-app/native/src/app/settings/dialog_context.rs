use super::*;

impl App {
    pub(super) fn dialog_context(&self, dialog: &SettingsDialog) -> Context {
        let mut ctx = self.settings_context(dialog.section, dialog.copied, dialog.trash_page);
        ctx.sync_selected = dialog.sync_selected.clone();
        ctx.sync_notice = dialog.sync_notice.clone();
        ctx.server_advanced = dialog.server_advanced;
        ctx
    }
}
