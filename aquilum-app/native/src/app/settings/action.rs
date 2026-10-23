use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};

use super::bindings::same_setter;
use super::{App, Effect, Outcome, Popup};
use crate::ui::dialog::DialogAnswer;
use crate::ui::editor::{TextAction, TextEditor};
use crate::ui::menu::MenuChoice;
use crate::ui::settings::Binding;
use crate::ui::settings::controls::{Changed, DropdownButton, Picked};
use crate::ui::settings::schema::{Action, Value};
use crate::ui::widgets::Pressed;

impl App {
    pub(in crate::app) fn on_settings_action(&mut self, root: &mut RenderRoot, id: WidgetId, action: &ErasedAction) -> Outcome {
        self.typing = None;
        let Some(Popup::Settings { mut dialog, menu }) = self.popup.take() else { return Outcome::default() };
        let mut effect = Effect::None;
        // A confirmation dialog is a distinct focus-trapped interaction;
        // cancel/escape is ALWAYS inert. Never interpret a click behind the
        // active confirmation as permission to mutate journal metadata.
        if let Some((confirmation, decision)) = dialog.sync_confirm.take() {
            if let Some(answer) = confirmation.answer(id, action) {
                crate::ui::dialog::close(root, confirmation.layer);
                if answer == DialogAnswer::Confirm {
                    effect = self.confirm_sync_review(&mut dialog, decision);
                }
            } else {
                dialog.sync_confirm = Some((confirmation, decision));
            }
            return self.keep(dialog, None, effect, root);
        }

        // A cloud provider is a second, focus-trapped dialog above the settings
        // catalog. No click on the catalog may be interpreted while it is open.
        if let Some(mut cloud) = dialog.cloud_popup.take() {
            // A provider dialog owns a transient draft. Pressing Enter must
            // not silently persist it before the user clicks Save.
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
            let pressed = action.downcast_ref::<Pressed>().is_some();
            let closing = (id == cloud.layer && action.downcast_ref::<DialogAnswer>() == Some(&DialogAnswer::Cancel))
                || (pressed && (id == cloud.close || id == cloud.cancel));
            if closing {
                crate::ui::dialog::close(root, cloud.layer);
                root.focus_on(Some(dialog.layer));
                return self.keep(dialog, None, Effect::Content, root);
            }
            if pressed && cloud.save == Some(id) {
                // Save also commits the current contents of fields even if the
                // user clicked Save without pressing Enter in each field.
                let texts = cloud.bindings.iter().filter_map(|(id, binding)| {
                    let Binding::Text { setter, .. } = binding else { return None };
                    let text = root.get_widget(*id)?
                        .downcast::<TextEditor>()?.inner().text();
                    Some((*setter, text))
                }).collect::<Vec<_>>();
                // The compact S3 page exposes one ordinary text field: bucket.
                // Reject an invalid typed bucket instead of closing the popup
                // while the setter silently kept the previous valid bucket.
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
            let (mut effect, _) = self.on_binding(root, &mut cloud.bindings, id, action);
            if pressed && let Some(Binding::Button(button)) = cloud.bindings.get(&id) {
                let button = button.clone();
                // A saved provider never implicitly targets a different
                // bucket from the text currently visible in this dialog.
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

        if let Some(picked) = action.downcast_ref::<Picked>() {
            let Some(menu) = menu else {
                self.popup = Some(Popup::Settings { dialog, menu: None });
                return Outcome::default();
            };
            let hex = match picked {
                Picked::Color(hex) => {
                    self.preview_color(root, &mut dialog, menu.button, hex);
                    self.popup = Some(Popup::Settings { dialog, menu: Some(menu) });
                    return Outcome::default();
                }
                Picked::Closed(hex) => hex.clone(),
            };
            root.focus_on(Some(dialog.layer));
            if let Some(&Binding::Color(setter)) = dialog.bindings.get(&menu.button) {
                effect = self.apply(setter, Value::Text(hex));
            }
            return self.keep(dialog, None, effect, root);
        }

        if let Some(menu) = menu {
            if id == menu.button && matches!(action.downcast_ref::<Changed>(), Some(Changed::OpenColor(_))) {
                root.remove_layer(menu.layer);
                root.focus_on(Some(dialog.layer));
                if let Some(&Binding::Color(setter)) = dialog.bindings.get(&menu.button) {
                    let typed = dialog.bindings.values().find_map(|b| match b {
                        Binding::Text { setter: s, value, .. } if same_setter(*s, setter) => Some(value.clone()),
                        _ => None,
                    });
                    if let Some(hex) = typed {
                        effect = self.apply(setter, Value::Text(hex));
                    }
                }
                return self.keep(dialog, None, effect, root);
            }
            if id == menu.button && matches!(action.downcast_ref::<Changed>(), Some(Changed::OpenMenu(_))) {
                root.remove_layer(menu.layer);
                root.edit_widget(menu.button, |mut w| DropdownButton::set_open(&mut w.downcast(), false));
                root.focus_on(Some(dialog.layer));
                self.popup = Some(Popup::Settings { dialog, menu: None });
                return Outcome::default();
            }
            if action.downcast_ref::<MenuChoice>().is_some() {
                root.edit_widget(menu.button, |mut w| DropdownButton::set_open(&mut w.downcast(), false));
            }
            match action.downcast_ref::<MenuChoice>() {
                Some(MenuChoice::Item(index)) => {
                    root.remove_layer(menu.layer);
                    if let Some(Binding::Dropdown { setter, options, selected }) = dialog.bindings.get_mut(&menu.button) {
                        *selected = *index;
                        let label = options.get(*index).cloned().unwrap_or_default();
                        let setter = *setter;
                        root.edit_widget(menu.button, |mut w| DropdownButton::set_label(&mut w.downcast(), &label));
                        effect = self.apply(setter, Value::Index(*index));
                    }
                    root.focus_on(Some(dialog.layer));
                    self.popup = Some(Popup::Settings { dialog, menu: None });
                }
                Some(MenuChoice::Dismissed) => {
                    root.focus_on(Some(dialog.layer));
                    self.popup = Some(Popup::Settings { dialog, menu: None });
                }
                None | Some(MenuChoice::Hover(_) | MenuChoice::Sub(_)) => self.popup = Some(Popup::Settings { dialog, menu: Some(menu) }),
            }
            return self.finish(root, effect);
        }

        let closing = (id == dialog.layer && matches!(action.downcast_ref::<DialogAnswer>(), Some(DialogAnswer::Cancel)))
            || (id == dialog.close && action.downcast_ref::<Pressed>().is_some());
        if closing {
            self.yandex_probe.settings_closed();
            crate::ui::dialog::close(root, dialog.layer);
            return Outcome::default();
        }

        if action.downcast_ref::<Pressed>().is_some()
            && let Some(&(_, section)) = dialog.nav.iter().find(|(nav, _)| *nav == id)
        {
            self.select_section(root, &mut dialog, section);
            self.popup = Some(Popup::Settings { dialog, menu: None });
            return Outcome::default();
        }

        let (mut effect, menu) = self.on_binding(root, &mut dialog.bindings, id, action);
        if action.downcast_ref::<Pressed>().is_some()
            && let Some(Binding::Button(button)) = dialog.bindings.get(&id)
        {
            let button = button.clone();
            effect = match button {
                Action::InspectRecovery(operation_id) if dialog.section == super::Section::Sync =>
                    self.inspect_sync_incident(&mut dialog, &operation_id),
                Action::RequestRecoveryReview(decision) if dialog.section == super::Section::Sync =>
                    self.request_sync_review(root, &mut dialog, decision),
                Action::InspectRecovery(_) | Action::RequestRecoveryReview(_) => Effect::None,
                Action::OpenCloudProvider(provider) if dialog.section == super::Section::Sync => {
                    let config = self.workspace.core.settings.get_config();
                    let ctx = self.settings_context(dialog.section, dialog.copied, dialog.trash_page);
                    let (layer, popup) = crate::ui::settings::provider_modal(&config, &ctx, provider);
                    root.add_layer(layer, masonry::kurbo::Point::ORIGIN);
                    root.focus_on(Some(popup.layer));
                    dialog.cloud_popup = Some(popup);
                    Effect::None
                }
                other => self.press(&mut dialog, other),
            };
        }
        self.keep(dialog, menu, effect, root)
    }
}
