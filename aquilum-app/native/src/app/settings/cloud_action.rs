use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};

use super::{App, Effect, Outcome};
use crate::ui::dialog::DialogAnswer;
use crate::ui::editor::{TextAction, TextEditor};
use crate::ui::settings::{Binding, CloudPopup, SettingsDialog};
use crate::ui::settings::controls::Committed;
use crate::ui::settings::schema::{Action, Value};
use crate::ui::widgets::Pressed;

impl App {
    pub(super) fn on_cloud_action(
        &mut self, root: &mut RenderRoot, id: WidgetId, action: &ErasedAction,
        mut dialog: SettingsDialog, mut cloud: CloudPopup,
    ) -> Outcome {
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
            // Save commits typed fields even without an Enter keypress.
            let texts = cloud.bindings.iter().filter_map(|(id, binding)| {
                let Binding::Text { setter, .. } = binding else { return None };
                let text = root.get_widget(*id)?
                    .downcast::<TextEditor>()?.inner().text();
                Some((*setter, text))
            }).collect::<Vec<_>>();
            // Refuse invalid S3 buckets before closing the dialog.
            #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
            if texts.len() == 1 && !aquilum_core::settings::models::YandexS3Onboarding::valid_bucket(&texts[0].1) {
                dialog.yandex_notice = Some(crate::i18n::t("settings.yandex.keysNeedBucket"));
                dialog.cloud_popup = Some(cloud);
                return self.keep(dialog, None, Effect::Content, root);
            }
            let before = self.workspace.core.settings.get_config();
            let mut proposal = before.clone();
            for (setter, text) in &texts {
                if let crate::ui::settings::schema::Setter::Config(apply) = setter {
                    apply(&mut proposal, &Value::Text(text.clone()));
                }
            }
            if !aquilum_core::settings::models::YandexS3Onboarding::valid_bucket(
                &proposal.s3_onboarding.bucket,
            ) {
                dialog.yandex_notice = Some(crate::i18n::t("settings.yandex.keysNeedBucket"));
                dialog.cloud_popup = Some(cloud);
                return self.keep(dialog, None, Effect::Content, root);
            }
            let saved_bucket = proposal.s3_onboarding.bucket.clone();
            // This is one atomic settings transaction, not a series of
            // per-field saves which could leave partial configuration.
            if self.workspace.core.settings.update_config(proposal).is_err() {
                dialog.yandex_notice = Some(crate::i18n::t("settings.cloud.saveFailed"));
                dialog.cloud_popup = Some(cloud);
                return self.keep(dialog, None, Effect::Content, root);
            }
            if before.s3_onboarding.bucket != saved_bucket {
                self.yandex_probe.set_bucket(&saved_bucket);
                #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
                self.s3_pair.invalidate_verification();
            }
            crate::ui::dialog::close(root, cloud.layer);
            root.focus_on(Some(dialog.layer));
            return self.keep(dialog, None, Effect::Content, root);
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
            effect = self.press(&mut dialog, button);
        }
        dialog.cloud_popup = Some(cloud);
        return self.keep(dialog, None, effect, root);
    }
}
