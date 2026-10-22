use super::*;
use crate::ui::scroll::ScrollArea;
use crate::ui::widgets::ItemButton;

impl App {
    pub(in crate::app) fn open_settings(&mut self, root: &mut RenderRoot) {
        // A reopened dialog must never reuse a previous bucket/auth probe result.
        self.yandex_probe.settings_closed();
        let section = self.interface.settings_section().and_then(Section::from_key).unwrap_or(Section::Ui);
        self.show_settings(root, section);
    }

    pub(super) fn select_section(&mut self, root: &mut RenderRoot, dialog: &mut SettingsDialog, section: Section) {
        if dialog.section == section {
            return;
        }
        if dialog.section == Section::Sync {
            self.yandex_probe.settings_closed();
        }
        for &(nav, item) in &dialog.nav {
            root.edit_widget(nav, |mut w| ItemButton::set_active(&mut w.downcast(), item == section));
        }
        dialog.section = section;
        dialog.copied = None;
        dialog.trash_page = 0;
        dialog.sync_selected = None;
        dialog.sync_notice = None;
        dialog.yandex_notice = None;
        self.interface.set_settings_section(section.key());
        let config = self.workspace.core.settings.get_config();
        let ctx = self.settings_context(section, None, 0);
        let (content, bindings) = settings::content(&config, &ctx, section);
        dialog.bindings = bindings;
        dialog.content.edit(root, |mut area| {
            ScrollArea::set_child(&mut area, content);
            ScrollArea::set_offset(&mut area, 0.0);
        });
    }

}
