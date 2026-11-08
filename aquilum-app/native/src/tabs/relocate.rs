use super::{Path, Tabs};

impl Tabs {
    /// Preserve every open tab when a folder is moved, not only exact file names.
    pub fn rename(&mut self, from: &Path, to: &Path) -> bool {
        let mut changed = false;
        for tab in &mut self.tabs {
            if let Some(old) = tab.path.as_deref() {
                if let Ok(suffix) = old.strip_prefix(from) {
                    let moved = if suffix.as_os_str().is_empty() {
                        to.to_path_buf()
                    } else {
                        to.join(suffix)
                    };
                    tab.path = Some(moved);
                    changed = true;
                }
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_rename_rebases_nested_tabs_without_touching_similar_prefixes() {
        let original = Path::new("Folder");
        let moved = Path::new("Renamed");
        let first = original.join("first.md");
        let nested = original.join("nested").join("second.md");
        let other = Path::new("Folder-copy").join("untouched.md");
        let mut tabs = Tabs::default();
        tabs.open(first);
        tabs.open_new(nested);
        tabs.open_new(other.clone());

        assert!(tabs.rename(original, moved));
        assert_eq!(tabs.tabs()[0].path.as_deref(), Some(moved.join("first.md").as_path()));
        assert_eq!(tabs.tabs()[1].path.as_deref(), Some(moved.join("nested").join("second.md").as_path()));
        assert_eq!(tabs.tabs()[2].path.as_deref(), Some(other.as_path()));
    }
}
