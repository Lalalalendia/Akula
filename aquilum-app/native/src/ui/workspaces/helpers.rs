use super::*;

pub fn no_vault(error: Option<&str>) -> (NewWidget<Flex>, WidgetId) {
    let open = NewWidget::new(TextButton::new(t("common.openFolder"), Variant::Primary).with_icon(icons::FOLDER_OPEN));
    let id = open.id();
    (search::empty_state_with(icons::FOLDER_OPEN, &t("fileTree.noFolder"), error, Some(open.erased())), id)
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_lowercase();
    norm(a) == norm(b)
}

pub fn forget_texts(path: &Path) -> (String, String) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    (t("workspaces.forgetTitle"), t_with("workspaces.forgetDescription", &[("name", &name)]))
}
