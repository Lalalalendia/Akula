use super::*;

impl App {
    pub(super) fn press(&mut self, dialog: &mut SettingsDialog, action: Action) -> Effect {
        let core = &self.workspace.core;
        let workspace = self.tree.root().map(Path::to_path_buf);
        match action {
            // This handler is reached only for ordinary settings buttons.
            // Review actions require the separately confirmed modal flow.
            Action::InspectRecovery(_) | Action::RequestRecoveryReview(_) => Effect::None,
            Action::OpenTrash => {
                if let Some(workspace) = workspace {
                    match gate::ensure_trash(core, &workspace) {
                        Ok(path) => system::open(&path),
                        Err(error) => report!("корзина не открыта: {error:?}"),
                    }
                }
                Effect::None
            }
            Action::Restore(id) => {
                let Some(workspace) = workspace else { return Effect::None };
                if let Err(error) = gate::restore_deletion(core, &workspace, &id) {
                    report!("заметка не восстановлена: {error:?}");
                }
                Effect::Content
            }
            Action::TrashPage(page) => {
                dialog.trash_page = page;
                Effect::Content
            }
            Action::CreateTemplates => {
                let Some(workspace) = workspace else { return Effect::None };
                crate::app::templates::write_starters(core, &schema::templates_path(&workspace, &core.settings.get_config().templates.folder));
                Effect::Content
            }
            Action::CheckUpdates => {
                self.updater.check();
                Effect::Content
            }
            Action::InstallUpdate => {
                self.updater.install();
                Effect::Content
            }
            Action::NewToken => self.apply(Setter::Config(|c, _| c.mcp.token = schema::new_token()), Value::Bool(true)),
            Action::Copy(index, code) => {
                match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(code)) {
                    Ok(()) => dialog.copied = Some(index),
                    Err(error) => report!("не скопировано: {error}"),
                }
                Effect::Content
            }
            Action::PluginDemoInstall => {
                match plugins::install(&self.workspace.data_dir) {
                    Ok(()) => self.plugin_demo_update(|p| {
                        p.installed = true;
                        p.enabled = true;
                        p.last_words = None;
                        p.last_error = None;
                    }),
                    Err(error) => self.plugin_demo_update(|p| { p.last_words = None; p.last_error = Some(error); }),
                }
            }
            Action::PluginDemoRun => {
                match self.plugin_demo_run() {
                    Ok(words) => self.plugin_demo_update(|p| { p.last_words = Some(words as u64); p.last_error = None; }),
                    Err(error) => self.plugin_demo_update(|p| { p.last_words = None; p.last_error = Some(error); }),
                }
            }
            Action::PluginDemoDisable => self.plugin_demo_update(|p| {
                p.enabled = false;
                p.last_words = None;
                        p.last_error = None;
            }),
            Action::PluginDemoEnable => self.plugin_demo_update(|p| {
                p.enabled = true;
                p.last_words = None;
                        p.last_error = None;
            }),
            Action::PluginDemoRemove => {
                match plugins::uninstall(&self.workspace.data_dir) {
                    Ok(()) => self.plugin_demo_update(|p| {
                        p.installed = false;
                        p.enabled = false;
                        p.last_words = None;
                        p.last_error = None;
                    }),
                    Err(error) => self.plugin_demo_update(|p| { p.last_words = None; p.last_error = Some(error); }),
                }
            }
            Action::OpenUrl(url) => {
                system::open_url(url);
                Effect::None
            }
        }
    }

}
