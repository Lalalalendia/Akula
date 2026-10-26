use masonry::core::{NewWidget, PropertySet, WidgetId};
use masonry::layout::Length;
use masonry::properties::Gap;
use masonry::properties::types::{CrossAxisAlignment, MainAxisAlignment};
use masonry::widgets::Flex;

use super::super::tokens::size;
use super::super::widgets::LinkButton;
use crate::i18n;

pub(super) fn new_tab_view() -> (NewWidget<Flex>, WidgetId, WidgetId, WidgetId, Option<WidgetId>, Option<WidgetId>) {
    let create = LinkButton::new(&i18n::t("editor.createNote"));
    let open = LinkButton::new(&i18n::t("editor.openFile"));
    let close = LinkButton::new(&i18n::t("editor.close"));
    let (create_id, open_id, close_id) = (create.id(), open.id(), close.id());
    let mut column = Flex::column()
        .main_axis_alignment(MainAxisAlignment::Center)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .with_fixed(create)
        .with_fixed(open)
        .with_fixed(close);
    let (mut dicom_folder, mut dicom_file) = (None, None);
    #[cfg(feature = "dicom-preview")]
    {
        let folder = LinkButton::new(&i18n::t("dicom.openFolder"));
        let file = LinkButton::new(&i18n::t("dicom.openFile"));
        dicom_folder = Some(folder.id());
        dicom_file = Some(file.id());
        column = column.with_fixed(folder).with_fixed(file);
    }
    let column = NewWidget::new(column).with_props(PropertySet::new().with(Gap::new(Length::px(size::SPACE_6))));
    (column, create_id, open_id, close_id, dicom_folder, dicom_file)
}

#[cfg(all(test, feature = "dicom-preview"))]
mod dicom_locale_tests {
    #[test]
    fn both_locales_have_all_direct_open_labels() {
        for source in [include_str!("../../../assets/locales/ru.json"), include_str!("../../../assets/locales/en.json")] {
            let locale: serde_json::Value = serde_json::from_str(source).unwrap();
            for key in ["openFolder", "openFile", "openCurrentFolder", "openCurrentFile"] {
                assert!(locale["dicom"][key].as_str().is_some_and(|s| !s.is_empty()), "missing dicom.{key}");
            }
        }
    }
}
