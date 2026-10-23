use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};

use super::bindings::same_setter;
use super::{App, Effect, OpenMenu, Outcome, Popup};
use crate::ui::menu::MenuChoice;
use crate::ui::settings::{Binding, SettingsDialog};
use crate::ui::settings::controls::{Changed, DropdownButton};
use crate::ui::settings::schema::Value;

impl App {
    pub(super) fn on_menu_action(
        &mut self, root: &mut RenderRoot, id: WidgetId, action: &ErasedAction,
        mut dialog: SettingsDialog, menu: OpenMenu,
    ) -> Outcome {
        let mut effect = Effect::None;
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
}
