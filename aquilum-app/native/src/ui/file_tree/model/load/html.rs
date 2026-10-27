use super::*;

/// Opt-in enumeration: unchanged default and installed file trees.
pub(super) fn append_html_entries(dir: &Path, items: &mut Vec<FileItem>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        if !kind.is_file() || kind.is_symlink()
            || entry.file_name().to_string_lossy().starts_with('.')
            || aquilum_core::search::paths::is_hidden_name(&entry.file_name()) {
            continue;
        }
        let path = entry.path();
        if !crate::html_preview::is_html_path(&path) { continue; }
        let id = path.to_string_lossy().into_owned();
        if items.iter().any(|item| item.id == id) { continue; }
        items.push(FileItem {
            id,
            name: entry.file_name().to_string_lossy().into_owned(),
            item_type: FileItemType::File,
        });
    }
}

#[cfg(test)]
mod html_file_tree_tests {
    use super::*;
    #[test]
    fn html_visible_only_with_preview_feature() {
        let vault = tempfile::tempdir().unwrap();
        std::fs::write(vault.path().join("one.HTML"), b"<p>hi</p>").unwrap();
        std::fs::write(vault.path().join("two.htm"), b"<p>two</p>").unwrap();
        std::fs::write(vault.path().join(".hidden.html"), b"<p>secret</p>").unwrap();
        let mut items = Vec::new();
        append_html_entries(vault.path(), &mut items);
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|f| f.name == "one.HTML"));
        assert!(items.iter().any(|f| f.name == "two.htm"));
    }
}
