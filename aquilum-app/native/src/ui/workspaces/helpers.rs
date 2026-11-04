use super::*;

mod no_vault;
pub use no_vault::no_vault;

pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_lowercase();
    norm(a) == norm(b)
}

pub fn forget_texts(path: &Path) -> (String, String) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    (t("workspaces.forgetTitle"), t_with("workspaces.forgetDescription", &[("name", &name)]))
}
