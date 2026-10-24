use super::*;
use std::path::Path;
impl App {
    pub(super) fn apply(&mut self, setter: Setter, value: Value) -> Effect {
        match setter {
            Setter::Scale => {
                let Value::Number(percent) = value else { return Effect::None };
                if self.interface.set_scale(percent / 100.0) { Effect::Rescale } else { Effect::None }
            }
            Setter::Autostart => {
                let Value::Bool(on) = value else { return Effect::None };
                if !crate::autostart::set(on) {
                    report!("автозапуск не изменён");
                }
                Effect::Content
            }
            Setter::HomePage => {
                let (Value::Text(page), Some(root)) = (value, self.tree.root()) else { return Effect::None };
                if let Err(error) = self.workspace.core.ui_state.set_home_page(&root.to_string_lossy(), page.trim()) {
                    report!("домашняя страница не сохранена: {error:?}");
                }
                self.refresh_home();
                Effect::None
            }
            Setter::Config(set) => {
                let core = &self.workspace.core;
                let before = core.settings.get_config();
                let mut after = before.clone();
                set(&mut after, &value);
                let language_changed = before.ui.language != after.ui.language;
                let colors_changed = before.ui.theme != after.ui.theme || before.ui.primary_color != after.ui.primary_color;
                let fonts_changed = before.ui.font.font_family != after.ui.font.font_family
                    || before.ui.font.font_weight != after.ui.font.font_weight
                    || before.ui.font.font_size_base != after.ui.font.font_size_base
                    || before.editor.font.font_family != after.editor.font.font_family
                    || before.editor.font.font_weight != after.editor.font.font_weight
                    || before.editor.font.font_size_base != after.editor.font.font_size_base
                    || before.editor.line_height != after.editor.line_height
                    || before.editor.max_width_ch != after.editor.max_width_ch;
                let mcp_changed = serde_json::to_value(&before.mcp).ok() != serde_json::to_value(&after.mcp).ok();
                let retention = before.trash.retention_days != after.trash.retention_days || before.history.retention_days != after.history.retention_days;
                let json = |c: &aquilum_core::settings::models::AppConfig| serde_json::to_value((&c.analysis, &c.search)).ok();
                let analysis_changed = json(&before) != json(&after);
                let methods_toggled = crate::links::Method::enabled(&before).is_empty() != crate::links::Method::enabled(&after).is_empty();
                crate::ui::motion::set(after.ui.animations);
                let rows_changed = before.ui.animations != after.ui.animations
                    || before.analysis.enable_bm25f != after.analysis.enable_bm25f
                    || before.updates.auto != after.updates.auto
                    || before.templates.folder != after.templates.folder
                    || mcp_changed
                    || retention
                    || before.s3_onboarding != after.s3_onboarding
                    || before.server_setup != after.server_setup;
                let new_yandex_bucket = after.s3_onboarding.bucket.clone();
                let dropbox_changed = before.dropbox_readonly != after.dropbox_readonly;
                if let Err(error) = core.settings.update_config(after) {
                    report!("настройки не сохранены: {error}");
                    return Effect::None;
                }
                #[cfg(feature = "dropbox-readonly-preview")]
                if dropbox_changed { self.dropbox_preview.invalidate(); }
                if before.s3_onboarding.bucket != new_yandex_bucket {
                    self.yandex_probe.set_bucket(&new_yandex_bucket);
                    #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
                    self.s3_pair.invalidate_verification();
                }
                if mcp_changed {
                    core.apply_mcp_settings();
                }
                if retention {
                    self.cleanup_retention();
                }
                if language_changed || methods_toggled {
                    Effect::Everything
                } else if fonts_changed || colors_changed {
                    Effect::Restyle(fonts_changed)
                } else if rows_changed || dropbox_changed {
                    Effect::Content
                } else if analysis_changed {
                    Effect::Panel
                } else {
                    Effect::None
                }
            }
        }
    }
    pub(super) fn refresh_analysis(&mut self, root: &mut RenderRoot) {
        if self.panel_mode() == crate::links::Mode::Analysis {
            self.request_panel(root);
        }
    }
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
