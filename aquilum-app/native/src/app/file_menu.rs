mod popup;
mod open;
#[cfg(feature = "dicom-preview")]
mod dicom;

use super::*;
use crate::ui::menu::{Submenu, Swatch};
use crate::ui::palette;

const MENU_PDF_PREVIEW: usize = 3;
const MENU_PDF_PICK: usize = 4;
#[cfg(feature = "dicom-preview")]
const MENU_DICOM_OPEN: usize = 5;
#[cfg(feature = "dicom-preview")]
const MENU_DICOM_PICK_FOLDER: usize = 6;
#[cfg(feature = "dicom-preview")]
const MENU_DICOM_PICK_FILE: usize = 7;
#[cfg(feature = "dicom-preview")]
const MENU_DICOM_FILE: usize = 3;
#[cfg(feature = "dicom-preview")]
use dicom::readonly_dicom_target;

impl App {
    pub(super) fn open_sort_menu(&mut self, root: &mut RenderRoot, button: WidgetId) {
        let Some(trigger) = root.get_widget(button).map(|w| w.ctx().bounding_box()) else { return };
        let labels: Vec<String> = crate::ui::file_tree::Sort::ALL.iter().map(|s| t(s.label())).collect();
        let items: Vec<&str> = labels.iter().map(String::as_str).collect();
        let checked = crate::ui::file_tree::Sort::ALL.iter().position(|s| *s == self.tree.sort()).unwrap_or(0);
        let origin = Menu::list_origin(trigger, items.len(), self.window);
        let menu = Menu::list(&t("fileTree.sort"), &items, checked, trigger);
        let layer = menu.id();
        root.add_layer(menu, origin);
        root.edit_widget(button, |mut w| crate::ui::widgets::IconButton::set_pressed(&mut w.downcast(), true));
        root.focus_on(Some(layer));
        self.popup = Some(Popup::SortMenu { layer, button });
    }

    pub(super) fn open_tabs_menu(&mut self, root: &mut RenderRoot, button: WidgetId) {
        let Some(trigger) = root.get_widget(button).map(|w| w.ctx().bounding_box()) else { return };
        let titles: Vec<String> = self.tabs.tabs().iter().map(|tab| if tab.graph { t("tabs.graph") } else { tab.title().unwrap_or_else(|| t("tabs.newTab")) }).collect();
        let items: Vec<&str> = titles.iter().map(String::as_str).collect();
        let checked = self.tabs.tabs().iter().position(|tab| tab.id == self.tabs.active_id()).unwrap_or(0);
        let width = crate::ui::tokens::size::TAB_LIST_WIDTH;
        let origin = Menu::dropdown_origin(trigger, self.window, width);
        let menu = Menu::dropdown(&t("tabs.list"), &items, checked, trigger, self.window, width);
        let layer = menu.id();
        root.add_layer(menu, origin);
        root.edit_widget(button, |mut w| crate::ui::widgets::IconButton::set_pressed(&mut w.downcast(), true));
        root.focus_on(Some(layer));
        self.popup = Some(Popup::TabsMenu { layer, button });
    }

    fn dropdown_answer(&mut self, root: &mut RenderRoot, layer: WidgetId, button: WidgetId, id: WidgetId, action: &ErasedAction) -> Option<Option<usize>> {
        let release = |root: &mut RenderRoot| root.edit_widget(button, |mut w| crate::ui::widgets::IconButton::set_pressed(&mut w.downcast(), false));
        if id == button && action.downcast_ref::<crate::ui::widgets::Pressed>().is_some() {
            root.remove_layer(layer);
            release(root);
            return Some(None);
        }
        let choice = action.downcast_ref::<MenuChoice>()?;
        if let MenuChoice::Item(_) = choice {
            root.remove_layer(layer);
        }
        release(root);
        Some(match choice {
            MenuChoice::Item(index) => Some(*index),
            MenuChoice::Dismissed | MenuChoice::Hover(_) | MenuChoice::Sub(_) => None,
        })
    }

    fn pick_tab(&mut self, root: &mut RenderRoot, index: usize) -> Outcome {
        let Some(id) = self.tabs.tabs().get(index).map(|tab| tab.id) else { return Outcome::default() };
        self.on_titlebar(root, &crate::ui::titlebar::TitlebarAction::Select(id), self.window)
    }

}
