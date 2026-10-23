use masonry::app::RenderRoot;

use super::bindings::{resume_switches, same_setter, switch_travel};
use super::{App, Popup};
use crate::ui::dialog::Modal;
use crate::ui::editor::TextEditor;
use crate::ui::scroll::ScrollArea;
use crate::ui::settings::{self, Binding, SettingsDialog};

impl App {
    pub(super) fn reload_settings(&mut self, root: &mut RenderRoot) {
        let Some(Popup::Settings { dialog, menu }) = self.popup.take() else { return };
        if let Some(menu) = menu {
            root.remove_layer(menu.layer);
        }
        let offset = dialog.content.get(root).map(|area| area.inner().offset());
        let switches = switch_travel(root, &dialog.bindings);
        let config = self.workspace.core.settings.get_config();
        let ctx = self.dialog_context(&dialog);
        let (card, finish) = settings::card(&config, &ctx, dialog.section);
        root.edit_widget(dialog.layer, |mut w| Modal::set_card(&mut w.downcast(), card, &crate::i18n::t("settings.title")));
        let mut fresh = finish(dialog.layer);
        fresh.sync_selected = dialog.sync_selected;
        fresh.sync_notice = dialog.sync_notice;
        fresh.yandex_notice = dialog.yandex_notice;
        fresh.server_advanced = dialog.server_advanced;
        fresh.cloud_popup = dialog.cloud_popup;
        if let Some(cloud) = fresh.cloud_popup.as_mut() {
            cloud.rebuild(root, &config, &ctx);
        }
        if fresh.cloud_popup.is_none() { root.focus_on(Some(fresh.layer)); }
        if let Some(offset) = offset {
            fresh.content.edit(root, |mut area| ScrollArea::set_offset(&mut area, offset));
        }
        resume_switches(root, &fresh.bindings, &switches);
        self.popup = Some(Popup::Settings { dialog: fresh, menu: None });
        self.refocus(root);
    }

    fn refocus(&mut self, root: &mut RenderRoot) {
        let Some((setter, text, caret)) = self.typing.take() else { return };
        let Some(Popup::Settings { dialog, .. }) = &self.popup else { return };
        let find_field = |bindings: &std::collections::HashMap<masonry::core::WidgetId, Binding>| bindings.iter()
            .find_map(|(id, binding)| match binding {
                Binding::Number { setter: s, .. } | Binding::Text { setter: s, .. }
                    if same_setter(*s, setter) => Some(*id),
                _ => None,
            });
        let field = dialog.cloud_popup.as_ref()
            .and_then(|cloud| find_field(&cloud.bindings))
            .or_else(|| find_field(&dialog.bindings));
        if let Some(field) = field {
            root.focus_on(Some(field));
            root.edit_widget(field, |mut w| {
                let mut editor = w.downcast::<TextEditor>();
                TextEditor::reset_text(&mut editor, &text);
                let caret = caret.min(text.len());
                TextEditor::select_byte_range(&mut editor, caret, caret);
            });
        }
    }

    pub(in crate::app) fn refresh_settings(&mut self, root: &mut RenderRoot) {
        let Some(Popup::Settings { dialog, .. }) = &self.popup else { return };
        let section = dialog.section;
        let config = self.workspace.core.settings.get_config();
        let ctx = self.dialog_context(dialog);
        let (content, bindings) = settings::content(&config, &ctx, section);
        let Some(Popup::Settings { dialog, .. }) = &mut self.popup else { return };
        let switches = switch_travel(root, &dialog.bindings);
        dialog.bindings = bindings;
        dialog.content.edit(root, |mut area| ScrollArea::set_child(&mut area, content));
        if let Some(cloud) = dialog.cloud_popup.as_mut() {
            cloud.rebuild(root, &config, &ctx);
        }
        resume_switches(root, &dialog.bindings, &switches);
        self.refocus(root);
    }
}
