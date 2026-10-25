use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};
use super::{App, Effect, Outcome};
use crate::ui::dialog::DialogAnswer;
use crate::ui::editor::{TextAction, TextEditor};
use crate::ui::settings::{Binding, CloudPopup, SettingsDialog};
use crate::ui::settings::controls::Committed;
use crate::ui::settings::schema::{Action, Value};
use crate::ui::widgets::Pressed; mod save; #[cfg(feature = "notion-offline-preview")] mod notion_draft;
impl App {
    pub(super) fn on_cloud_action(
        &mut self, root: &mut RenderRoot, id: WidgetId, action: &ErasedAction,
        mut dialog: SettingsDialog, mut cloud: CloudPopup,
    ) -> Outcome {
        #[cfg(feature = "notion-offline-preview")]
        if self.capture_notion_draft(root,id,action,&mut cloud) {dialog.cloud_popup=Some(cloud);return self.keep(dialog,None,Effect::None,root);}
        if let Some(text_action) = action.downcast_ref::<TextAction>()
            && let Some(Binding::Text { setter, .. }) = cloud.bindings.get(&id)
        {
            match text_action {
                TextAction::Changed(text) | TextAction::Entered(text) => {
                    cloud.remember_text(*setter, text);
                }
                _ => {}
            }
            dialog.cloud_popup = Some(cloud);
            return self.keep(dialog, None, Effect::None, root);
        }
        if action.downcast_ref::<Committed>().is_some() {
            let draft = match cloud.bindings.get(&id) {
                Some(Binding::Commit(field)) => {
                    let setter = match cloud.bindings.get(&field.id()) {
                        Some(Binding::Text { setter, .. }) => Some(*setter),
                        _ => None,
                    };
                    setter.and_then(|setter| {
                        field.get(root).map(|editor| (setter, editor.inner().text()))
                    })
                }
                _ => None,
            };
            if let Some((setter, text)) = draft {
                cloud.remember_text(setter, &text);
            }
            dialog.cloud_popup = Some(cloud);
            return self.keep(dialog, None, Effect::None, root);
        }
        let pressed = action.downcast_ref::<Pressed>().is_some();
        let closing = (id == cloud.layer && action.downcast_ref::<DialogAnswer>() == Some(&DialogAnswer::Cancel))
            || (pressed && (id == cloud.close || id == cloud.cancel));
        if closing {
            crate::ui::dialog::close(root, cloud.layer);
            root.focus_on(Some(dialog.layer));
            return self.keep(dialog, None, Effect::Content, root);
        }
        if pressed && cloud.save == Some(id) {
            return self.save_cloud_popup(root, dialog, cloud);
        }
        #[cfg(feature = "notion-offline-preview")]
        if pressed && let Some(Binding::Button(button)) = cloud.bindings.get(&id)
            && self.handle_notion_action(button)
        {
            let config = self.workspace.core.settings.get_config();
            let ctx = self.settings_context(dialog.section, dialog.copied, dialog.trash_page);
            cloud.rebuild(root, &config, &ctx);
            dialog.cloud_popup = Some(cloud);
            return self.keep(dialog, None, Effect::None, root);
        }
        let (mut effect, _) = self.on_binding(root, &mut cloud.bindings, id, action);
        if pressed && let Some(Binding::Button(button)) = cloud.bindings.get(&id) {
            let button = button.clone();
            #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
            if cloud.provider == crate::ui::settings::schema::CloudProvider::Yandex {
                let draft = cloud.bindings.iter().find_map(|(field, binding)| {
                    let Binding::Text { setter, .. } = binding else { return None };
                    let text = root.get_widget(*field)?
                        .downcast::<TextEditor>()?.inner().text();
                    Some((*setter, text))
                });
                if let Some((setter, bucket)) = draft {
                    let valid = aquilum_core::settings::models::YandexS3Onboarding::valid_bucket(&bucket);
                    let stored = self.workspace.core.settings.get_config().s3_onboarding.bucket;
                    let uses_bucket = matches!(button,
                        Action::StoreYandexKeys | Action::RemoveYandexKeys |
                        Action::ConfigureS3Pair | Action::ExportS3Pair |
                        Action::ImportS3Pair | Action::SyncS3PairOnce
                    );
                    if uses_bucket && !valid {
                        dialog.yandex_notice = Some(crate::i18n::t("settings.yandex.keysNeedBucket"));
                        dialog.cloud_popup = Some(cloud);
                        return self.keep(dialog, None, Effect::Content, root);
                    }
                    if uses_bucket && bucket != stored {
                        if matches!(button, Action::StoreYandexKeys) {
                            // The explicit OS credential dialog must never
                            // open with the *previous* bucket on failed
                            // settings persistence.
                            let before = self.workspace.core.settings.get_config();
                            let mut next = before.clone();
                            if let crate::ui::settings::schema::Setter::Config(apply) = setter {
                                apply(&mut next, &Value::Text(bucket.clone()));
                            }
                            if next.s3_onboarding.bucket != bucket {
                                dialog.yandex_notice = Some(crate::i18n::t("settings.yandex.keysNeedBucket"));
                                dialog.cloud_popup = Some(cloud);
                                return self.keep(dialog, None, Effect::Content, root);
                            }
                            if self.workspace.core.settings.update_config(next).is_err() {
                                dialog.yandex_notice = Some(crate::i18n::t("settings.cloud.saveFailed"));
                                dialog.cloud_popup = Some(cloud);
                                return self.keep(dialog, None, Effect::Content, root);
                            }
                            self.yandex_probe.set_bucket(&bucket);
                            self.s3_pair.invalidate_verification();
                        } else {
                            dialog.yandex_notice = Some(crate::i18n::t("settings.cloud.saveBeforeAction"));
                            dialog.cloud_popup = Some(cloud);
                            return self.keep(dialog, None, Effect::Content, root);
                        }
                    }
                }
            }
            #[cfg(feature = "dropbox-readonly-preview")]
            if cloud.provider == crate::ui::settings::schema::CloudProvider::Dropbox
                && cloud.has_draft_changes()
                && matches!(&button, Action::DropboxConnectReadOnly | Action::DropboxResumeReadOnly)
            {
                self.dropbox_preview.status = "DROPBOX_SAVE_ADVANCED_FIRST / SYNC_DISABLED".into();
                dialog.cloud_popup = Some(cloud);
                return self.keep(dialog, None, Effect::Content, root);
            }
            effect = self.press(&mut dialog, button);
        }
        dialog.cloud_popup = Some(cloud);
        return self.keep(dialog, None, effect, root);
    }
}
