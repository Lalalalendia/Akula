mod popup;

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
fn readonly_dicom_target(clicked: &Path, targets: &[PathBuf]) -> bool {
    crate::dicom::is_candidate(clicked) || targets.iter().any(|path| crate::dicom::is_candidate(path))
}


impl App {
    pub(super) fn open_file_menu(
        &mut self,
        root: &mut RenderRoot,
        targets: Vec<PathBuf>,
        clicked: PathBuf,
        folder: bool,
        at: Point,
        window: Size,
        tab: Option<Uuid>,
    ) {
        let label = if tab.is_some() {
            t("tabs.actions")
        } else if folder {
            t("fileTree.folderActions")
        } else {
            t("fileTree.fileActions")
        };
        let mut labels = vec![(t("common.rename"), crate::ui::icons::PENCIL), (t("common.duplicate"), crate::ui::icons::COPY), (t("common.delete"), crate::ui::icons::TRASH)];
        if tab.is_some() {
            labels.push((t("tabs.close"), crate::ui::icons::X));
        } else if !folder && targets.len() == 1 && pdf_preview::is_pdf_path(&clicked) {
            labels.push(("Найти текст PDF в Akula".to_owned(), crate::ui::icons::SEARCH));
        }
        #[cfg(feature = "dicom-preview")]
        if !folder && tab.is_none() && targets.len() == 1 && crate::dicom::is_candidate(&clicked) {
            labels.push(("Открыть DICOM без импорта".to_owned(), crate::ui::icons::SEARCH));
        }
        let colored = folder && tab.is_none();
        if colored {
            labels.push((t("palette.color"), crate::ui::icons::PALETTE));
            labels.push(("Импортировать PDF в Akula".to_owned(), crate::ui::icons::UPLOAD));
            #[cfg(feature = "dicom-preview")]
            {
                labels.push(("DICOM: открыть эту папку напрямую".to_owned(), crate::ui::icons::SEARCH));
                labels.push(("DICOM: выбрать внешнюю папку".to_owned(), crate::ui::icons::FOLDER));
                labels.push(("DICOM: выбрать ZIP или файл".to_owned(), crate::ui::icons::SEARCH));
            }
        }
        #[cfg(feature = "dicom-preview")]
        let protect = readonly_dicom_target(&clicked, &targets);
        #[cfg(not(feature = "dicom-preview"))]
        let protect = false;
        // Medical sources are previewed in place. Do not offer the usual
        // Rename/Duplicate/Delete commands that mutate their originals.
        let items: Vec<crate::ui::menu::Entry<'_>> = labels.iter().enumerate()
            .map(|(at, (label, icon))| crate::ui::menu::Entry {
                label, icon: Some(*icon), disabled: protect && at <= MENU_DELETE,
            }).collect();
        let names: Vec<String> = std::iter::once(t("palette.none")).chain(palette::NAMES.iter().map(|name| palette::label(name))).collect();
        let theme = crate::ui::theme::current();
        let swatches: Vec<Swatch<'_>> = names.iter().zip(std::iter::once(None).chain(palette::NAMES.iter().map(|name| palette::color(name, &theme)))).map(|(label, color)| Swatch { label, color }).collect();
        let current = self.tree.color(&clicked).and_then(|name| palette::NAMES.iter().position(|known| *known == name)).map_or(0, |at| at + 1);
        let menu = if colored { Menu::with_submenu(&label, &items, &Submenu { at: MENU_COLOR, items: &swatches, checked: current }) } else { Menu::entries(&label, &items) };
        let layer = menu.id();
        root.add_layer(menu, Menu::context_origin(at, items.len(), window));
        root.focus_on(Some(layer));
        self.popup = Some(Popup::FileMenu { layer, targets, clicked, tab });
    }

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

#[cfg(all(test, feature = "dicom-preview"))]
mod dicom_menu_readonly_tests {
    use super::*;

    #[test]
    fn medical_zip_dicomdir_and_multiselect_cannot_offer_destructive_actions() {
        let base = tempfile::tempdir().unwrap();
        let study = base.path().join("study");
        std::fs::create_dir(&study).unwrap();
        std::fs::write(study.join("DICOMDIR"), [0u8; 132]).unwrap();
        assert!(readonly_dicom_target(&study, &[]));
        assert!(readonly_dicom_target(Path::new("case.zip"), &[]));
        assert!(readonly_dicom_target(Path::new("slice.DCM"), &[]));
        let candidates = vec![PathBuf::from("note.md"), PathBuf::from("other.dcm")];
        assert!(readonly_dicom_target(Path::new("note.md"), &candidates));
        assert!(!readonly_dicom_target(Path::new("ordinary.md"), &[]));
    }
}
