use super::*;

pub fn no_vault(error: Option<&str>) -> (NewWidget<Flex>, WidgetId, Option<WidgetId>, Option<WidgetId>) {
    let open = NewWidget::new(TextButton::new(t("common.openFolder"), Variant::Primary).with_icon(icons::FOLDER_OPEN));
    let id = open.id();
    let mut controls = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .with_fixed(open);
    let (mut folder_id, mut file_id) = (None, None);
    #[cfg(feature = "dicom-preview")]
    {
        let folder = NewWidget::new(TextButton::new("DICOM: открыть папку без импорта", Variant::Ghost).with_icon(icons::FOLDER_OPEN));
        let file = NewWidget::new(TextButton::new("DICOM: открыть ZIP или файл", Variant::Ghost).with_icon(icons::FILE));
        folder_id = Some(folder.id());
        file_id = Some(file.id());
        controls = controls.with_fixed(folder).with_fixed(file);
    }
    let controls = NewWidget::new(controls).with_props(
        masonry::core::PropertySet::new().with(
            Gap::new(Length::px(size::SPACE_4))
        )
    );
    (search::empty_state_with(
        icons::FOLDER_OPEN, &t("fileTree.noFolder"), error, Some(controls.erased())
    ), id, folder_id, file_id)
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_lowercase();
    norm(a) == norm(b)
}

pub fn forget_texts(path: &Path) -> (String, String) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    (t("workspaces.forgetTitle"), t_with("workspaces.forgetDescription", &[("name", &name)]))
}
