use super::*;

impl App {
    pub(in crate::app) fn open_file_menu(
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
            labels.push((t("dicom.openCurrentFile"), crate::ui::icons::SEARCH));
        }
        let colored = folder && tab.is_none();
        if colored {
            labels.push((t("palette.color"), crate::ui::icons::PALETTE));
            labels.push(("Импортировать PDF в Akula".to_owned(), crate::ui::icons::UPLOAD));
            #[cfg(feature = "dicom-preview")]
            {
                labels.push((t("dicom.openCurrentFolder"), crate::ui::icons::SEARCH));
                labels.push((t("dicom.openFolder"), crate::ui::icons::FOLDER));
                labels.push((t("dicom.openFile"), crate::ui::icons::SEARCH));
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

}
