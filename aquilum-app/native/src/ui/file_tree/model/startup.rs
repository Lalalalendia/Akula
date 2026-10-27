use std::path::PathBuf;

use super::{Sort, TreeModel, comparable};

impl TreeModel {
    // Keep the legacy metadata/sorting route for time-based sort modes.
    // Only NameDesc demonstrated a consistent Windows startup improvement.
    pub fn with_saved_sort(root: Option<PathBuf>, expanded: impl IntoIterator<Item = PathBuf>, sort: Sort) -> Self {
        if sort == Sort::NameDesc {
            return Self::with_sort(root, expanded, sort);
        }
        let mut model = Self::new(root, expanded);
        model.set_sort(sort);
        model
    }

    pub fn with_sort(root: Option<PathBuf>, expanded: impl IntoIterator<Item = PathBuf>, sort: Sort) -> Self {
        let expanded = expanded.into_iter().map(|p| (comparable(&p), p)).collect();
        let mut model = TreeModel { root, expanded, sort, ..Default::default() };
        model.reload_all();
        model
    }
}
