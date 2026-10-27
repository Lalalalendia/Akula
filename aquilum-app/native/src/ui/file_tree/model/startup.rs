use std::path::PathBuf;

use super::{Sort, TreeModel, comparable};

impl TreeModel {
    pub fn with_sort(root: Option<PathBuf>, expanded: impl IntoIterator<Item = PathBuf>, sort: Sort) -> Self {
        let expanded = expanded.into_iter().map(|p| (comparable(&p), p)).collect();
        let mut model = TreeModel { root, expanded, sort, ..Default::default() };
        model.reload_all();
        model
    }
}
