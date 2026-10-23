use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};

use super::{App, Effect, Outcome, Popup};
use crate::ui::dialog::DialogAnswer;
use crate::ui::settings::Binding;
use crate::ui::settings::controls::Picked;
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

        // A provider modal owns input until it is explicitly closed.
        if let Some(cloud) = dialog.cloud_popup.take() {
            return self.on_cloud_action(root, id, action, dialog, cloud);
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
            return self.on_menu_action(root, id, action, dialog, menu);
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
