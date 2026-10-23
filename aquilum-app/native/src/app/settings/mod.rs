use std::collections::HashMap;
use std::path::Path;

use aquilum_core::files::gate;
use crate::ui::editor::TextEditor;
use masonry::app::RenderRoot;
use masonry::core::keyboard::{Key, NamedKey};
use masonry::core::{ErasedAction, WidgetId};
use masonry::kurbo::Point;
use crate::ui::editor::TextAction;

use super::{App, Outcome, Popup};

mod action;
mod apply;
mod context;
mod bindings;
mod refresh;
mod recovery;
mod retention;
mod dialog_context;
mod types;
pub(super) use types::Effect;
pub(super) mod server_state;
#[cfg(all(windows, feature = "server-readonly-preflight"))]
pub(super) mod server;

use crate::system;
use crate::ui::menu::Menu;
use crate::ui::settings::controls::{Changed, ColorPicker, Committed, DropdownButton, Switch};
use crate::ui::theme;
use crate::ui::tokens::size;
use crate::ui::settings::schema::{self, Action, Context, Section, Setter, Value};
use crate::ui::settings::{self, Binding, SettingsDialog};
use crate::ui::widgets::Pressed;

pub(super) struct OpenMenu {
    pub(super) layer: WidgetId,
    button: WidgetId,
}


impl App {
    pub(super) fn open_settings(&mut self, root: &mut RenderRoot) {
        let section = self.interface.settings_section().and_then(Section::from_key).unwrap_or(Section::Ui);
        self.show_settings(root, section);
    }

    fn show_settings(&mut self, root: &mut RenderRoot, section: Section) {
        let config = self.workspace.core.settings.get_config();
        let ctx = self.settings_context(section, None, 0);
        let (layer, dialog) = settings::dialog(&config, &ctx, section);
        root.add_layer(layer, Point::ORIGIN);
        root.focus_on(Some(dialog.layer));
        self.popup = Some(Popup::Settings { dialog, menu: None });
    }

    pub(super) fn rebuild(&mut self, root: &mut RenderRoot) {
        self.replace_root(root);
        self.reload_settings(root);
    }

    fn keep(&mut self, dialog: SettingsDialog, menu: Option<OpenMenu>, effect: Effect, root: &mut RenderRoot) -> Outcome {
        self.popup = Some(Popup::Settings { dialog, menu });
        self.finish(root, effect)
    }

    fn finish(&mut self, root: &mut RenderRoot, effect: Effect) -> Outcome {
        if let Some(ids) = &self.note_ids {
            let smart = crate::ui::editor::Smart::from_config(&self.workspace.core.settings.get_config());
            ids.body.edit(root, |mut body| crate::ui::editor::TextEditor::set_smart(&mut body, smart));
        }
        match effect {
            Effect::None => Outcome::default(),
            Effect::Rescale => Outcome { rescale: true, ..Outcome::default() },
            Effect::Panel => {
                self.refresh_analysis(root);
                Outcome::default()
            }
            Effect::Content => {
                self.refresh_settings(root);
                self.refresh_analysis(root);
                Outcome::default()
            }
            Effect::Restyle(fonts) => {
                self.restyle(root, fonts);
                Outcome::default()
            }
            Effect::Everything => {
                self.rebuild(root);
                Outcome::default()
            }
        }
    }

    pub fn settings_key(&mut self, root: &mut RenderRoot, key: &Key) -> Option<Outcome> {
        let direction = match key {
            Key::Named(NamedKey::ArrowUp) => 1.0,
            Key::Named(NamedKey::ArrowDown) => -1.0,
            _ => return None,
        };
        let field = root.focused_widget()?;
        let &Binding::Number { setter, .. } = self.popup_bindings()?.get(&field)? else { return None };
        let mut bindings = std::mem::take(self.popup_bindings()?);
        let effect = self.step_number(root, &mut bindings, field, direction);
        if let Some(Binding::Number { number, .. }) = bindings.get(&field) {
            let text = schema::format_number(number.value);
            self.typing = Some((setter, text.clone(), text.len()));
        }
        if let Some(slot) = self.popup_bindings() {
            *slot = bindings;
        }
        if let Some(Popup::Reader(state)) = &self.popup {
            let settings = crate::app::reader::reader_settings(&self.workspace.core.settings.get_config());
            root.edit_widget(state.layer, |mut w| crate::ui::reader::ReaderView::set_settings(&mut w.downcast(), settings));
        }
        Some(self.finish(root, effect))
    }

    fn popup_bindings(&mut self) -> Option<&mut HashMap<WidgetId, Binding>> {
        match &mut self.popup {
            Some(Popup::Settings { dialog, menu: None }) => Some(&mut dialog.bindings),
            Some(Popup::Reader(state)) => Some(&mut state.bindings),
            _ => None,
        }
    }
}
