use super::*;

pub(super) fn trace(root: Option<&Path>, dir: &Path, items: &[FileItem]) {
    if !crate::host::native_frame() { return; }
    let txt = items.iter().filter(|item| {
        item.item_type == FileItemType::File && aquilum_core::search::paths::is_plain_text(Path::new(&item.id))
    }).count();
    if txt > 0 || root == Some(dir) {
        report!("WIN7_TREE_SCAN root={} entries={} txt={}", root == Some(dir), items.len(), txt);
    }
}
